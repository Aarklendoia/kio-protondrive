//! Periodic background refresh for `core::cache`'s permanent filesystem
//! stat/listing cache (issue #8) — the counterpart to `bridge.rs`'s
//! read-through/write-through population on a genuine cache miss.
//!
//! That cache has no TTL: once a path is cached, `protondrive:/` browsing
//! serves it instantly forever, however old, until either (a) this app's
//! own writes invalidate what they touch, or (b) this module's periodic
//! sweep re-fetches it. Sequential, not parallel — each `proton-drive` CLI
//! call already costs ~1-4s on its own (measured live), so fanning many out
//! at once would just contend with itself and with any concurrent KIO
//! worker activity.
//!
//! After a refresh, fires `org.kde.KDirNotify.FilesChanged` for the
//! refreshed path(s) so any Dolphin window with that folder open re-renders
//! — confirmed live elsewhere in this project (see `control.rs`'s
//! `notify_pin_changed` doc comment) that this specific signal does make
//! Dolphin visibly re-stat/re-list, unlike the untested `FilesAdded` — and,
//! separately, `notify_pin_changed` itself (despite the name, a generic
//! "an overlay-relevant field changed" broadcast, see its own doc comment)
//! for each refreshed path, since `FilesChanged` alone is confirmed *not*
//! to repaint `worker/overlayplugin.cpp`'s pin/local-cache/sharing badges —
//! without this, a sharing change made outside this project (e.g. Proton's
//! web app) would only reach the "shared" badge after this sweep updated
//! `fs_stat_cache`, never visibly, until some *other* unrelated overlay
//! event happened to repaint the same icon.
//! Best-effort, same stance as every other side-channel notification here:
//! no session bus (e.g. inside a container) just means the view goes stale
//! until the next natural refresh, not a failure worth surfacing.

use std::process::Command;

use protondrive_core::cache::Cache;
use protondrive_core::cli::{self, CommandRunner, DriveError};
use protondrive_core::entry::ListItem;

use crate::control::notify_pin_changed;

fn drive_url(path: &str) -> String {
    format!("protondrive:{path}")
}

/// `dbus-send`'s syntax for a `QStringList` argument.
fn dbus_string_array(values: &[String]) -> String {
    let quoted: Vec<String> = values.iter().map(|v| format!("string:\"{v}\"")).collect();
    format!("array:{}", quoted.join(","))
}

fn notify_files_changed(urls: &[String]) {
    if urls.is_empty() {
        return;
    }
    let result = Command::new("dbus-send")
        .arg("--session")
        .arg("--type=signal")
        .arg("/")
        .arg("org.kde.KDirNotify.FilesChanged")
        .arg(dbus_string_array(urls))
        .status();
    if let Err(err) = result {
        log::debug!("could not send FilesChanged for {urls:?} (dbus-send missing?): {err}");
    }
}

/// Re-fetches every cached stat path and listing, replacing what's cached
/// (or, on a confirmed `DriveError::NotFound`, dropping it — the remote is
/// gone, no point keeping a stale record around for the next sweep to redo).
/// Any other error (auth, timeout, transient CLI hiccup) leaves that entry
/// as-is for the next cycle rather than losing it over a blip.
pub fn refresh_all(runner: &dyn CommandRunner, cache: &Cache) {
    let stat_paths = match cache.all_cached_stat_paths() {
        Ok(paths) => paths,
        Err(err) => {
            log::warn!("fs cache refresh: could not list cached stat paths: {err}");
            Vec::new()
        }
    };
    for path in stat_paths {
        match cli::stat_path(runner, &path) {
            Ok(node) => {
                let _ = cache.store_stat(&path, &node);
                notify_files_changed(&[drive_url(&path)]);
                notify_pin_changed(&path);
            }
            Err(DriveError::NotFound(_)) => {
                let _ = cache.invalidate_stat(&path);
            }
            Err(err) => {
                log::debug!("fs cache refresh: stat {path} failed, keeping stale entry: {err}");
            }
        }
    }

    let listing_parents = match cache.all_cached_listing_parents() {
        Ok(parents) => parents,
        Err(err) => {
            log::warn!("fs cache refresh: could not list cached listing parents: {err}");
            Vec::new()
        }
    };
    for parent in listing_parents {
        match cli::list_dir(runner, &parent) {
            Ok(items) => {
                let nodes: Vec<_> = items
                    .into_iter()
                    .filter_map(|item| match item {
                        ListItem::Node(node) => Some(node),
                        ListItem::Section(_) => None,
                    })
                    .collect();
                let child_paths: Vec<String> = nodes
                    .iter()
                    .map(|n| format!("{}/{}", parent.trim_end_matches('/'), n.display_name()))
                    .collect();
                let urls: Vec<String> = child_paths.iter().map(|p| drive_url(p)).collect();
                let _ = cache.store_listing(&parent, &nodes);
                notify_files_changed(&urls);
                for child_path in &child_paths {
                    notify_pin_changed(child_path);
                }
            }
            Err(DriveError::NotFound(_)) => {
                let _ = cache.invalidate_listing(&parent);
            }
            Err(err) => {
                log::debug!(
                    "fs cache refresh: listing {parent} failed, keeping stale entry: {err}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protondrive_core::cli::CommandOutput;
    use protondrive_core::entry::{DecryptedField, NodeEntry};
    use std::cell::RefCell;
    use std::time::Duration;

    /// Returns canned JSON per exact argument list, in insertion order for
    /// repeats of the same args — same shape as `cli.rs`'s own test doubles.
    struct ScriptedRunner {
        responses: RefCell<Vec<(Vec<String>, CommandOutput)>>,
    }

    impl ScriptedRunner {
        fn new(responses: Vec<(Vec<&str>, &str)>) -> Self {
            Self {
                responses: RefCell::new(
                    responses
                        .into_iter()
                        .map(|(args, stdout)| {
                            (
                                args.into_iter().map(str::to_string).collect(),
                                CommandOutput {
                                    stdout: stdout.to_string(),
                                    stderr: String::new(),
                                    success: true,
                                },
                            )
                        })
                        .collect(),
                ),
            }
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let mut responses = self.responses.borrow_mut();
            let pos = responses
                .iter()
                .position(|(expected, _)| expected == &args)
                .unwrap_or_else(|| panic!("unscripted call: {args:?}"));
            Ok(responses.remove(pos).1)
        }
    }

    fn node_json(uid: &str, name: &str) -> String {
        format!(
            r#"{{"uid":"{uid}","name":{{"ok":true,"value":"{name}"}},"type":"file","isShared":false,"creationTime":"2026-01-01T00:00:00.000Z","modificationTime":"2026-01-01T00:00:00.000Z"}}"#
        )
    }

    fn node(uid: &str, name: &str) -> NodeEntry {
        NodeEntry {
            uid: uid.to_string(),
            name: DecryptedField {
                ok: true,
                value: Some(name.to_string()),
            },
            node_type: "file".to_string(),
            media_type: None,
            total_storage_size: Some(123),
            creation_time: "2026-01-01T00:00:00.000Z".to_string(),
            modification_time: "2026-01-01T00:00:00.000Z".to_string(),
            is_shared: false,
            is_shared_by_url: false,
            photo: None,
        }
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            Cache::open(&dir.path().join("index.sqlite3"), &dir.path().join("files")).unwrap();
        (dir, cache)
    }

    #[test]
    fn refreshes_a_cached_stat_path() {
        let (_dir, cache) = cache();
        cache
            .store_stat("/my-files/a.txt", &node("uid-1", "a.txt"))
            .unwrap();
        let runner = ScriptedRunner::new(vec![(
            vec!["filesystem", "info", "-j", "/my-files/a.txt"],
            &node_json("uid-1", "a-renamed.txt"),
        )]);

        refresh_all(&runner, &cache);

        let refreshed = cache.cached_stat("/my-files/a.txt").unwrap().unwrap();
        assert_eq!(refreshed.display_name(), "a-renamed.txt");
    }

    #[test]
    fn drops_a_stat_entry_whose_remote_is_gone() {
        let (_dir, cache) = cache();
        cache
            .store_stat("/my-files/gone.txt", &node("uid-1", "gone.txt"))
            .unwrap();
        struct NotFoundRunner;
        impl CommandRunner for NotFoundRunner {
            fn run(&self, _args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
                Err(DriveError::NotFound("/my-files/gone.txt".to_string()))
            }
        }

        refresh_all(&NotFoundRunner, &cache);

        assert!(cache.cached_stat("/my-files/gone.txt").unwrap().is_none());
    }

    #[test]
    fn refreshes_a_cached_listing() {
        let (_dir, cache) = cache();
        cache
            .store_listing("/my-files", &[node("uid-1", "a.txt")])
            .unwrap();
        let runner = ScriptedRunner::new(vec![
            (
                vec!["filesystem", "list", "-j", "/my-files"],
                &format!("[{}]", node_json("uid-1", "a.txt")),
            ),
            (
                vec!["filesystem", "info", "-j", "/my-files/a.txt"],
                &node_json("uid-1", "a.txt"),
            ),
        ]);

        refresh_all(&runner, &cache);

        let refreshed = cache.cached_listing("/my-files").unwrap().unwrap();
        assert_eq!(refreshed.len(), 1);
    }
}
