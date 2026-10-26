//! Periodic background refresh for `core::cache`'s permanent filesystem
//! stat/listing cache (issue #8) — the counterpart to `bridge.rs`'s
//! read-through/write-through population on a genuine cache miss.
//!
//! That cache has no TTL: once a path is cached, `protondrive:/` browsing
//! serves it instantly, however old, until (a) this app's own writes
//! invalidate what they touch, (b) the worker's request to revalidate a
//! listing it just served comes back different ([`revalidate_listing`]),
//! or (c) this module's periodic sweep re-fetches it. Sequential, not parallel — each `proton-drive` CLI
//! call already costs ~1-4s on its own (measured live), so fanning many out
//! at once would just contend with itself and with any concurrent KIO
//! worker activity.
//!
//! After a refresh, fires `org.kde.KDirNotify.FilesChanged` for the
//! refreshed path(s) so any Dolphin window with that folder open re-renders
//! — confirmed live elsewhere in this project (see `control.rs`'s
//! `notify_overlay_changed` doc comment) that this specific signal does make
//! Dolphin visibly re-stat/re-list, unlike the untested `FilesAdded` — and,
//! separately, a single batched `notify_paths_changed` call (`control.rs`'s
//! own "an overlay-relevant field changed" broadcast, see its doc comment)
//! for every refreshed path at once, since `FilesChanged` alone is
//! confirmed *not* to repaint `worker/overlayplugin.cpp`'s
//! pin/local-cache/sharing badges —
//! without this, a sharing change made outside this project (e.g. Proton's
//! web app) would only reach the "shared" badge after this sweep updated
//! `fs_stat_cache`, never visibly, until some *other* unrelated overlay
//! event happened to repaint the same icon.
//! Best-effort, same stance as every other side-channel notification here:
//! no session bus (e.g. inside a container) just means the view goes stale
//! until the next natural refresh, not a failure worth surfacing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use protondrive_core::cache::Cache;
use protondrive_core::cli::{self, CommandRunner, DriveError};
use protondrive_core::entry::{ListItem, NodeEntry};

use crate::control::{emit_string, emit_string_list, notify_paths_changed};

/// How long after revalidating a folder another request for it is ignored.
/// Without it, the re-list Dolphin does after [`revalidate_listing`]'s own
/// `FilesAdded` would be served from the cache again, ask for another
/// revalidation, and so on for as long as the folder stays open.
const REVALIDATE_COOLDOWN: Duration = Duration::from_secs(10);

/// When each folder was last revalidated, for [`REVALIDATE_COOLDOWN`].
/// Recorded *before* the CLI call, so a second request arriving while the
/// first is still running is dropped too.
static LAST_REVALIDATED: Mutex<BTreeMap<String, Instant>> = Mutex::new(BTreeMap::new());

fn drive_url(path: &str) -> String {
    format!("protondrive:{path}")
}

fn notify_files_changed(urls: &[String]) {
    if urls.is_empty() {
        return;
    }
    emit_string_list("org.kde.KDirNotify", "FilesChanged", urls);
}

fn fetch_listing(runner: &dyn CommandRunner, parent: &str) -> Result<Vec<NodeEntry>, DriveError> {
    Ok(cli::list_dir(runner, parent)?
        .into_iter()
        .filter_map(|item| match item {
            ListItem::Node(node) => Some(node),
            ListItem::Section(_) => None,
        })
        .collect())
}

fn child_paths(parent: &str, nodes: &[NodeEntry]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| format!("{}/{}", parent.trim_end_matches('/'), n.display_name()))
        .collect()
}

/// Whether `parent` is due for a revalidation, recording it as revalidated
/// now if so. Also forgets entries older than the cooldown, so the map only
/// ever holds the last few seconds' worth of folders.
fn claim_revalidation(parent: &str) -> bool {
    let mut last = LAST_REVALIDATED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    last.retain(|_, at| at.elapsed() < REVALIDATE_COOLDOWN);
    if last.contains_key(parent) {
        return false;
    }
    last.insert(parent.to_string(), Instant::now());
    true
}

/// Stale-while-revalidate for one folder: the worker has just served
/// `parent`'s listing from the cache (instantly, possibly stale) and asked
/// for it to be checked against Drive. If what Drive has now differs from
/// what was served, the cache is updated and Dolphin told to list the folder
/// again — so a change made elsewhere shows up a few seconds after opening
/// or refreshing (F5) the folder, instead of after the next
/// [`refresh_all`] sweep.
///
/// `FilesAdded(<folder>)`, not [`refresh_all`]'s per-child `FilesChanged`:
/// KIO re-lists the whole folder on it, which also picks up children that
/// were added or removed, and still works on a folder that was empty.
/// Nothing is emitted when the listing didn't change, so the re-list it
/// triggers ends the cycle (along with [`REVALIDATE_COOLDOWN`]).
pub fn revalidate_listing(runner: &dyn CommandRunner, cache: &Cache, parent: &str) -> bool {
    if !claim_revalidation(parent) {
        return false;
    }
    let previous = cache.cached_listing(parent).ok().flatten();
    match fetch_listing(runner, parent) {
        Ok(nodes) => {
            if previous.as_deref() == Some(nodes.as_slice()) {
                return false;
            }
            let _ = cache.store_listing(parent, &nodes);
            emit_string("org.kde.KDirNotify", "FilesAdded", &drive_url(parent));
            notify_paths_changed(&child_paths(parent, &nodes));
            true
        }
        Err(DriveError::NotFound(_)) => {
            let _ = cache.invalidate_listing(parent);
            false
        }
        Err(err) => {
            log::debug!("revalidate: listing {parent} failed, keeping cached entry: {err}");
            false
        }
    }
}

/// Re-fetches every cached stat path and listing, replacing what's cached
/// (or, on a confirmed `DriveError::NotFound`, dropping it — the remote is
/// gone, no point keeping a stale record around for the next sweep to redo).
/// Any other error (auth, timeout, transient CLI hiccup) leaves that entry
/// as-is for the next cycle rather than losing it over a blip.
///
/// Every genuinely refreshed path (from either loop below) is collected
/// into `changed_paths` and broadcast once, deduplicated, via a single
/// batched `notify_paths_changed` call at the end — not per-path inline —
/// since `store_listing` already primes `fs_stat_cache` for every child of
/// a listing (see its own doc comment), so most children of a cached
/// folder show up in *both* loops on the same sweep, and a large cache
/// (hundreds of paths) would otherwise mean hundreds of individual
/// broadcast subprocess spawns per sweep on this daemon's single main
/// loop.
pub fn refresh_all(runner: &dyn CommandRunner, cache: &Cache) {
    let mut changed_paths: BTreeSet<String> = BTreeSet::new();

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
                changed_paths.insert(path);
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
        match fetch_listing(runner, &parent) {
            Ok(nodes) => {
                let child_paths = child_paths(&parent, &nodes);
                let urls: Vec<String> = child_paths.iter().map(|p| drive_url(p)).collect();
                let _ = cache.store_listing(&parent, &nodes);
                notify_files_changed(&urls);
                changed_paths.extend(child_paths);
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

    notify_paths_changed(&changed_paths.into_iter().collect::<Vec<_>>());
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
            active_revision: None,
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

    fn list_runner(parent: &str, names: &[&str]) -> ScriptedRunner {
        let json = names
            .iter()
            .enumerate()
            .map(|(i, name)| node_json(&format!("uid-{i}"), name))
            .collect::<Vec<_>>()
            .join(",");
        ScriptedRunner::new(vec![(
            vec!["filesystem", "list", "-j", parent],
            &format!("[{json}]"),
        )])
    }

    // Each test uses its own folder: the cooldown map is process-wide.

    #[test]
    fn revalidate_stores_a_listing_that_changed_remotely() {
        let (_dir, cache) = cache();
        cache
            .store_listing("/reval-changed", &[node("uid-0", "a.txt")])
            .unwrap();
        let runner = list_runner("/reval-changed", &["a.txt", "new.txt"]);

        assert!(revalidate_listing(&runner, &cache, "/reval-changed"));

        let names: Vec<String> = cache
            .cached_listing("/reval-changed")
            .unwrap()
            .unwrap()
            .iter()
            .map(|n| n.display_name().to_string())
            .collect();
        assert_eq!(names, ["a.txt", "new.txt"]);
    }

    #[test]
    fn revalidate_reports_no_change_for_an_identical_listing() {
        let (_dir, cache) = cache();
        let runner = list_runner("/reval-same", &["a.txt"]);
        let nodes = fetch_listing(&runner, "/reval-same").unwrap();
        cache.store_listing("/reval-same", &nodes).unwrap();

        let runner = list_runner("/reval-same", &["a.txt"]);
        assert!(!revalidate_listing(&runner, &cache, "/reval-same"));
    }

    #[test]
    fn revalidate_ignores_a_folder_within_its_cooldown() {
        let (_dir, cache) = cache();
        cache
            .store_listing("/reval-cooldown", &[node("uid-0", "a.txt")])
            .unwrap();
        let runner = list_runner("/reval-cooldown", &["b.txt"]);
        assert!(revalidate_listing(&runner, &cache, "/reval-cooldown"));

        // Unscripted: a second CLI call would panic.
        let runner = ScriptedRunner::new(vec![]);
        assert!(!revalidate_listing(&runner, &cache, "/reval-cooldown"));
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
