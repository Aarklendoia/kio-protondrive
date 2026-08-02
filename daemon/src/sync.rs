//! Local -> Drive upload logic for **pinned** files (see issue #30 —
//! `protondrive_core::cache`). One function, [`upload_if_needed`], is
//! reused both by [`reconcile`] (a directory walk done once at startup, to
//! catch anything that changed while the daemon wasn't running) and by the
//! live watcher loop in `main.rs` — the two are the same operation, just
//! driven by different triggers.
//!
//! Unlike the old folder-pair design, there's no single configured local
//! root to derive a remote destination from: each local file under the
//! (fixed) cache root maps to its own remote path via
//! `Cache::lookup_by_local_path`, recorded when it was pinned. A local file
//! under the cache root with no matching pin record isn't something this
//! daemon put there or knows how to sync — silently ignored rather than
//! treated as an error (e.g. a stray temp file some other process created
//! in the cache directory).
//!
//! Failures are logged and skipped rather than propagated: the pin
//! record for a failed file is left untouched, so it's naturally retried
//! on the next reconcile pass or the next time it's touched — no separate
//! retry/backoff queue, per docs/DESIGN.md. The one exception is a
//! missing/expired session (`DriveError::NotAuthenticated`): every
//! remaining file would fail identically, so [`reconcile`] propagates that
//! instead of logging it once per file — `main.rs` is responsible for
//! reacting to it (see `DaemonError::is_authentication_error`).

use std::path::Path;
use std::time::UNIX_EPOCH;

use protondrive_core::cache::Cache;
use protondrive_core::cli::{self, CommandRunner, DriveError};

use crate::error::DaemonError;

/// The remote path a local cache file would have if it were pinned at
/// exactly this location — derived by stripping the cache root prefix,
/// since the cache directory structure mirrors Drive's. Used for the
/// *new* side of a rename, which has no pin record yet to look up.
fn remote_path_for_local(cache: &Cache, local_path: &Path) -> Option<String> {
    let rel = local_path.strip_prefix(cache.root()).ok()?;
    Some(format!("/{}", rel.to_string_lossy()))
}

fn remote_parent(remote_path: &str) -> String {
    match remote_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => remote_path[..idx].to_string(),
    }
}

/// Propagates a local rename/move of a pinned file's cache copy (`from` ->
/// `to`, both under the cache root) to Drive, instead of leaving the
/// remote copy under its old name with no local counterpart tracked
/// anymore.
///
/// If `from` isn't a currently pinned local path, there's nothing to
/// propagate — silently ignored (matches [`upload_if_needed`]'s handling
/// of untracked cache-root files).
pub fn handle_rename(
    runner: &dyn CommandRunner,
    cache: &Cache,
    from: &Path,
    to: &Path,
) -> Result<(), DaemonError> {
    let Some(old_remote_path) = cache.lookup_by_local_path(from)? else {
        return Ok(());
    };
    let Some(new_remote_path) = remote_path_for_local(cache, to) else {
        return Ok(());
    };

    let remote_result = (|| -> Result<(), DriveError> {
        let old_remote_parent = remote_parent(&old_remote_path);
        let new_remote_parent = remote_parent(&new_remote_path);
        let new_name = to.file_name().unwrap_or_default().to_string_lossy();

        cli::ensure_remote_dir_chain(runner, &new_remote_parent)?;
        if old_remote_parent == new_remote_parent {
            cli::rename_path(runner, &old_remote_path, &new_name)?;
        } else {
            cli::move_path(runner, &old_remote_path, &new_remote_parent)?;
            let old_name = from.file_name().unwrap_or_default().to_string_lossy();
            if old_name != new_name {
                let moved_path = format!("{new_remote_parent}/{old_name}");
                cli::rename_path(runner, &moved_path, &new_name)?;
            }
        }
        Ok(())
    })();

    match remote_result {
        Ok(()) => cache
            .rename(&old_remote_path, &new_remote_path, to)
            .map_err(DaemonError::from),
        Err(err) => {
            // Couldn't propagate the rename remotely — still rekey the
            // local side so future edits to `to` keep syncing (to the OLD
            // remote path, which still exists under its old name): trades
            // a stale remote name for not silently dropping the file from
            // sync forever (a future reconcile walk only ever sees `to`
            // on disk, never `from` again).
            log::warn!(
                "failed to rename/move {} -> {} on Drive ({err}), keeping the remote copy under \
                 its old name",
                from.display(),
                to.display()
            );
            cache
                .rename(&old_remote_path, &old_remote_path, to)
                .map_err(DaemonError::from)
        }
    }
}

/// Uploads `local_file` if it's a pinned file whose content changed since
/// it was last recorded as synced, skipping it otherwise. Not-a-file paths
/// (e.g. a directory create event) and local files with no pin record are
/// silently ignored.
pub fn upload_if_needed(
    runner: &dyn CommandRunner,
    cache: &Cache,
    local_file: &Path,
) -> Result<(), DaemonError> {
    let metadata = match std::fs::metadata(local_file) {
        Ok(metadata) => metadata,
        // Common with debounced events: a temp file was created and then
        // renamed/removed before we got around to processing it.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    if !metadata.is_file() {
        return Ok(());
    }

    let Some(remote_path) = cache.lookup_by_local_path(local_file)? else {
        return Ok(());
    };

    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let size = metadata.len() as i64;

    if !cache.needs_upload(&remote_path, mtime, size)? {
        return Ok(());
    }

    let remote_parent = remote_parent(&remote_path);
    cli::ensure_remote_dir_chain(runner, &remote_parent)?;
    cli::upload(runner, local_file, &remote_parent)?;
    cache.mark_synced(&remote_path, mtime, size)?;
    Ok(())
}

/// Walks the cache root recursively, uploading every pinned file that
/// needs it. Run once at startup to catch local edits to pinned files made
/// while the daemon wasn't running; the live watcher loop then takes over
/// via the same [`upload_if_needed`].
pub fn reconcile(runner: &dyn CommandRunner, cache: &Cache) -> Result<(), DaemonError> {
    walk_and_upload(runner, cache, cache.root())
}

fn walk_and_upload(
    runner: &dyn CommandRunner,
    cache: &Cache,
    dir: &Path,
) -> Result<(), DaemonError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk_and_upload(runner, cache, &path)?;
        } else if file_type.is_file() {
            if let Err(err) = upload_if_needed(runner, cache, &path) {
                // Every remaining file would fail identically until the user
                // re-authenticates — stop the whole walk immediately instead
                // of burning a CLI call (and a log line) per file. Propagates
                // up through the recursive calls above via `?`.
                if err.is_authentication_error() {
                    return Err(err);
                }
                log::warn!(
                    "failed to sync {}: {err} (will retry next cycle)",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::time::Duration;

    use super::*;

    struct ScriptedRunner {
        responses: RefCell<VecDeque<cli::CommandOutput>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedRunner {
        fn new(responses: Vec<cli::CommandOutput>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<cli::CommandOutput, DriveError> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|s| s.to_string()).collect());
            Ok(self
                .responses
                .borrow_mut()
                .pop_front()
                .expect("no more scripted responses"))
        }
    }

    fn success(stdout: &str) -> cli::CommandOutput {
        cli::CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            success: true,
        }
    }

    fn failure(stderr: &str) -> cli::CommandOutput {
        cli::CommandOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
            success: false,
        }
    }

    const NODE_JSON: &str = r#"{
        "uid":"uid-1",
        "name":{"ok":true,"value":"Backups"},
        "type":"folder",
        "isShared":false,
        "creationTime":"2026-01-01T00:00:00.000Z",
        "modificationTime":"2026-01-01T00:00:00.000Z"
    }"#;

    const TRANSFER_OK: &str =
        r#"{"transferredItems":1,"transferredBytes":10,"skippedItems":0,"failedItems":0}"#;

    /// A single-node JSON response shaped like `rename_path`'s real output.
    const RENAMED_NODE_JSON: &str = r#"{
        "uid":"uid-1",
        "name":{"ok":true,"value":"new-name.pdf"},
        "type":"file",
        "isShared":false,
        "creationTime":"2026-01-01T00:00:00.000Z",
        "modificationTime":"2026-01-01T00:00:00.000Z"
    }"#;

    /// A `[{uid, ok}]` response shaped like `move_path`'s real output.
    const MOVE_OK: &str = r#"[{"uid":"uid-1","ok":true}]"#;

    /// Seeds a pin record directly (bypassing a real `pin()`/download call)
    /// — writes `local_path` to disk with `content` and records it in
    /// `cache` as if it had already been synced.
    fn seed_pin(cache: &Cache, remote_path: &str, local_path: &Path, content: &[u8]) {
        std::fs::create_dir_all(local_path.parent().unwrap()).unwrap();
        std::fs::write(local_path, content).unwrap();
        let metadata = std::fs::metadata(local_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        // Route through pin() so the DB row exists with the right
        // local_path, using a scripted runner that just echoes the file
        // already on disk back as the "download" — simpler than hand-
        // writing the INSERT here and risking the schema drifting apart
        // from Cache::pin's own.
        struct SeedRunner<'a> {
            local_path: &'a Path,
        }
        impl CommandRunner for SeedRunner<'_> {
            fn run(
                &self,
                args: &[&str],
                _timeout: Duration,
            ) -> Result<cli::CommandOutput, DriveError> {
                assert_eq!(args[0..2], ["filesystem", "download"]);
                let target_dir = args[args.len() - 1];
                let file_name = self.local_path.file_name().unwrap();
                std::fs::copy(self.local_path, Path::new(target_dir).join(file_name)).unwrap();
                Ok(cli::CommandOutput {
                    stdout: r#"{"transferredItems":1,"transferredBytes":1,"skippedItems":0,"failedItems":0,"failures":[]}"#.to_string(),
                    stderr: String::new(),
                    success: true,
                })
            }
        }
        cache.pin(&SeedRunner { local_path }, remote_path).unwrap();
        let _ = mtime; // already reflected via the real file's metadata pin() reads
    }

    fn cache(dir: &Path) -> Cache {
        Cache::open(&dir.join("index.sqlite3"), &dir.join("files")).unwrap()
    }

    #[test]
    fn upload_if_needed_ignores_a_local_file_with_no_pin_record() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let stray = cache.root().join("stray.txt");
        std::fs::create_dir_all(cache.root()).unwrap();
        std::fs::write(&stray, b"not pinned").unwrap();

        let runner = ScriptedRunner::new(Vec::new());
        upload_if_needed(&runner, &cache, &stray).unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn upload_if_needed_skips_a_pinned_file_already_synced() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let local_path = cache.root().join("my-files/report.pdf");
        seed_pin(&cache, "/my-files/report.pdf", &local_path, b"hello");

        let runner = ScriptedRunner::new(Vec::new());
        upload_if_needed(&runner, &cache, &local_path).unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn upload_if_needed_uploads_a_pinned_file_that_changed() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let local_path = cache.root().join("my-files/report.pdf");
        seed_pin(&cache, "/my-files/report.pdf", &local_path, b"hello");
        std::fs::write(&local_path, b"hello, edited").unwrap();

        // ensure_remote_dir_chain's single "info" call on /my-files succeeds
        // (already exists — it's a virtual root section, so actually a
        // no-op: /my-files alone has no segments beyond the root), then the
        // upload itself.
        let runner = ScriptedRunner::new(vec![success(TRANSFER_OK)]);
        upload_if_needed(&runner, &cache, &local_path).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0..3], ["filesystem", "upload", "-j"]);

        let metadata = std::fs::metadata(&local_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(!cache
            .needs_upload("/my-files/report.pdf", mtime, metadata.len() as i64)
            .unwrap());
    }

    #[test]
    fn handle_rename_ignores_a_from_path_with_no_pin_record() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let from = cache.root().join("my-files/untracked.txt");
        let to = cache.root().join("my-files/still-untracked.txt");

        let runner = ScriptedRunner::new(Vec::new());
        handle_rename(&runner, &cache, &from, &to).unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn handle_rename_renames_in_place_for_a_same_directory_rename() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let from = cache.root().join("my-files/old-name.pdf");
        let to = cache.root().join("my-files/new-name.pdf");
        seed_pin(&cache, "/my-files/old-name.pdf", &from, b"hello");

        let runner = ScriptedRunner::new(vec![success(RENAMED_NODE_JSON)]);
        handle_rename(&runner, &cache, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "filesystem",
                "rename",
                "-j",
                "/my-files/old-name.pdf",
                "new-name.pdf",
            ]
        );
        assert_eq!(cache.lookup("/my-files/old-name.pdf").unwrap(), None);
        assert_eq!(
            cache.lookup_by_local_path(&to).unwrap(),
            Some("/my-files/new-name.pdf".to_string())
        );
    }

    #[test]
    fn handle_rename_moves_for_a_cross_directory_move_with_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let from = cache.root().join("my-files/report.pdf");
        let to = cache.root().join("my-files/sub/report.pdf");
        seed_pin(&cache, "/my-files/report.pdf", &from, b"hello");

        // ensure_remote_dir_chain walks one segment beyond the virtual
        // root to reach /my-files/sub ("sub", not yet existing — created).
        let runner = ScriptedRunner::new(vec![
            failure("Path not found"),
            success(NODE_JSON),
            success(MOVE_OK),
        ]);
        handle_rename(&runner, &cache, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(
            calls[2],
            vec![
                "filesystem",
                "move",
                "-j",
                "/my-files/report.pdf",
                "/my-files/sub",
            ]
        );
        assert_eq!(
            cache.lookup_by_local_path(&to).unwrap(),
            Some("/my-files/sub/report.pdf".to_string())
        );
    }

    #[test]
    fn handle_rename_falls_back_to_keeping_the_old_remote_name_when_the_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let from = cache.root().join("my-files/old-name.pdf");
        let to = cache.root().join("my-files/new-name.pdf");
        seed_pin(&cache, "/my-files/old-name.pdf", &from, b"hello");

        let runner = ScriptedRunner::new(vec![failure("internal server error, please retry")]);
        handle_rename(&runner, &cache, &from, &to).unwrap();

        // Rekeyed to the new local path, but the remote path is unchanged
        // (the rename itself failed) — future edits to `to` still sync,
        // just to the old remote name.
        assert_eq!(
            cache.lookup_by_local_path(&to).unwrap(),
            Some("/my-files/old-name.pdf".to_string())
        );
    }

    #[test]
    fn reconcile_stops_the_whole_walk_on_the_first_authentication_failure() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let a = cache.root().join("my-files/a.pdf");
        let b = cache.root().join("my-files/b.pdf");
        seed_pin(&cache, "/my-files/a.pdf", &a, b"hello");
        seed_pin(&cache, "/my-files/b.pdf", &b, b"world");
        std::fs::write(&a, b"hello, edited").unwrap();
        std::fs::write(&b, b"world, edited").unwrap();

        let runner = ScriptedRunner::new(vec![failure("You need to login first")]);
        let err = reconcile(&runner, &cache).unwrap_err();
        assert!(err.is_authentication_error());
        // Only the first file's upload attempt happened — the walk stopped
        // immediately rather than also trying the second, which would fail
        // identically.
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}
