//! Keeps the `protondrive:` folders open in Dolphin up to date (#225).
//!
//! Since #221, a cached folder is revalidated when the worker serves it, so
//! opening it or pressing F5 shows remote changes a few seconds later. A
//! folder that just stays open, though, was only updated by
//! `fs_refresh::refresh_all`'s periodic sweep. KIO's directory listers
//! broadcast `org.kde.KDirNotify.enteredDirectory`/`leftDirectory` on the
//! session bus whenever a view starts or stops showing a folder: this module
//! follows them with `busctl --user monitor` (the daemon has no D-Bus client
//! library, it already emits through `busctl`, see `control.rs`), counts the
//! views showing each folder, and `main.rs` revalidates every open folder on
//! a short interval. `fs_refresh::revalidate_listing` only tells Dolphin
//! anything when the listing actually changed.
//!
//! A view that goes away without a `leftDirectory` (a crashed Dolphin) would
//! leave its folder counted forever, so an entry also expires [`EXPIRY`]
//! after it was last entered, or revalidated at the worker's request
//! ([`touch`]). Best-effort, like the rest of the daemon's bus use: no
//! session bus, or one that refuses to be monitored, just leaves the sweep as
//! the only refresh.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a folder stays tracked without a sign of life, see the module
/// docs. A folder genuinely left open longer than this with nobody touching
/// it falls back to the periodic sweep.
const EXPIRY: Duration = Duration::from_secs(60 * 60);

/// Match rules for `busctl monitor`, one per signal.
const MATCH_RULES: [&str; 2] = [
    "type='signal',interface='org.kde.KDirNotify',member='enteredDirectory'",
    "type='signal',interface='org.kde.KDirNotify',member='leftDirectory'",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Entered,
    Left,
}

#[derive(Debug)]
struct OpenFolder {
    views: u32,
    last_seen: Instant,
}

#[derive(Debug, Default)]
struct OpenFolders {
    folders: BTreeMap<String, OpenFolder>,
}

impl OpenFolders {
    const fn new() -> Self {
        Self {
            folders: BTreeMap::new(),
        }
    }

    fn apply(&mut self, change: Change, path: String, now: Instant) {
        match change {
            Change::Entered => {
                let folder = self.folders.entry(path).or_insert(OpenFolder {
                    views: 0,
                    last_seen: now,
                });
                folder.views += 1;
                folder.last_seen = now;
            }
            Change::Left => {
                if let Some(folder) = self.folders.get_mut(&path) {
                    folder.views = folder.views.saturating_sub(1);
                    if folder.views == 0 {
                        self.folders.remove(&path);
                    }
                }
            }
        }
    }

    /// Pushes back an open folder's expiry. Doesn't start tracking one.
    fn touch(&mut self, path: &str, now: Instant) {
        if let Some(folder) = self.folders.get_mut(path) {
            folder.last_seen = now;
        }
    }

    /// The folders still open, after dropping the expired ones.
    fn current(&mut self, now: Instant) -> Vec<String> {
        self.folders
            .retain(|_, folder| now.saturating_duration_since(folder.last_seen) < EXPIRY);
        self.folders.keys().cloned().collect()
    }
}

static OPEN_FOLDERS: Mutex<OpenFolders> = Mutex::new(OpenFolders::new());

fn open_folders() -> std::sync::MutexGuard<'static, OpenFolders> {
    OPEN_FOLDERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts following Dolphin's views in a background thread.
pub fn start() {
    let mut command = Command::new("busctl");
    command.args(["--user", "monitor", "--json=short"]);
    for rule in MATCH_RULES {
        command.args(["--match", rule]);
    }
    let mut child = match command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            log::debug!("not following open folders (busctl missing?): {err}");
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some((change, path)) = parse_signal(&line) {
                log::debug!("open folders: {change:?} {path}");
                open_folders().apply(change, path, Instant::now());
            }
        }
        let status = child.wait();
        log::debug!("stopped following open folders: busctl monitor exited ({status:?})");
    });
}

/// Pushes back `path`'s expiry if it's open: the worker just served it, so a
/// view is still showing it.
pub fn touch(path: &str) {
    open_folders().touch(path, Instant::now());
}

/// The drive paths of the folders currently open in a view.
pub fn current() -> Vec<String> {
    open_folders().current(Instant::now())
}

/// One `busctl monitor --json=short` line, e.g. `{"type":"signal",…,
/// "member":"enteredDirectory","payload":{"type":"s","data":
/// ["protondrive:/my-files"]}}`, as the change and the drive path, or `None`
/// for anything else, including folders outside `protondrive:`.
fn parse_signal(line: &str) -> Option<(Change, String)> {
    let message: serde_json::Value = serde_json::from_str(line).ok()?;
    let change = match message.get("member")?.as_str()? {
        "enteredDirectory" => Change::Entered,
        "leftDirectory" => Change::Left,
        _ => return None,
    };
    let url = message.get("payload")?.get("data")?.get(0)?.as_str()?;
    Some((change, drive_path(url)?))
}

/// `protondrive:/my-files/A%20B` → `/my-files/A B`. `None` for another
/// scheme, and for the folders `fs_refresh::revalidate_listing` can't
/// refresh: the root (sections, not nodes) and `/photos` (listed through
/// `photo timeline`, not `filesystem list`).
fn drive_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("protondrive:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let path = percent_decode(rest.trim_end_matches('/'));
    if !path.starts_with('/') || path == "/photos" || path.starts_with("/photos/") {
        return None;
    }
    Some(path)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(byte) = value
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(member: &str, url: &str) -> String {
        serde_json::json!({
            "type": "signal",
            "path": "/",
            "interface": "org.kde.KDirNotify",
            "member": member,
            "payload": {"type": "s", "data": [url]},
        })
        .to_string()
    }

    #[test]
    fn parse_signal_reads_entered_and_left_directory() {
        assert_eq!(
            parse_signal(&signal("enteredDirectory", "protondrive:/my-files")),
            Some((Change::Entered, "/my-files".to_string()))
        );
        assert_eq!(
            parse_signal(&signal("leftDirectory", "protondrive:/my-files/Docs/")),
            Some((Change::Left, "/my-files/Docs".to_string()))
        );
    }

    #[test]
    fn parse_signal_ignores_other_schemes_signals_and_unrefreshable_folders() {
        assert_eq!(
            parse_signal(&signal("enteredDirectory", "file:///home/me")),
            None
        );
        assert_eq!(
            parse_signal(&signal("FilesAdded", "protondrive:/my-files")),
            None
        );
        assert_eq!(
            parse_signal(&signal("enteredDirectory", "protondrive:/")),
            None
        );
        assert_eq!(
            parse_signal(&signal("enteredDirectory", "protondrive:/photos/favorites")),
            None
        );
        assert_eq!(parse_signal("not json"), None);
    }

    #[test]
    fn drive_path_decodes_percent_escapes() {
        assert_eq!(
            drive_path("protondrive:/my-files/A%20B/caf%C3%A9%25"),
            Some("/my-files/A B/café%".to_string())
        );
        assert_eq!(
            drive_path("protondrive:///my-files/x"),
            Some("/my-files/x".to_string())
        );
        assert_eq!(
            drive_path("protondrive:/my-files/100%"),
            Some("/my-files/100%".to_string())
        );
    }

    #[test]
    fn a_folder_stays_open_until_every_view_left_it() {
        let now = Instant::now();
        let mut open = OpenFolders::new();
        open.apply(Change::Entered, "/my-files".into(), now);
        open.apply(Change::Entered, "/my-files".into(), now);
        open.apply(Change::Left, "/my-files".into(), now);
        assert_eq!(open.current(now), vec!["/my-files".to_string()]);
        open.apply(Change::Left, "/my-files".into(), now);
        assert!(open.current(now).is_empty());
    }

    #[test]
    fn leaving_a_folder_that_was_never_entered_is_a_no_op() {
        let now = Instant::now();
        let mut open = OpenFolders::new();
        open.apply(Change::Left, "/my-files".into(), now);
        assert!(open.current(now).is_empty());
    }

    #[test]
    fn an_abandoned_folder_expires_unless_touched() {
        let start = Instant::now();
        let mut open = OpenFolders::new();
        open.apply(Change::Entered, "/my-files/a".into(), start);
        open.apply(Change::Entered, "/my-files/b".into(), start);
        open.touch("/my-files/b", start + EXPIRY / 2);
        // Touching a folder nobody entered doesn't start tracking it.
        open.touch("/my-files/c", start + EXPIRY / 2);

        assert_eq!(
            open.current(start + EXPIRY),
            vec!["/my-files/b".to_string()]
        );
    }
}
