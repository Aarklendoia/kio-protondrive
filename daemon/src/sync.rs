//! Local -> Drive upload logic. One function, [`upload_if_needed`], is
//! reused both by [`reconcile`] (a directory walk done once at startup, to
//! catch anything that changed while the daemon wasn't running) and by the
//! live watcher loop in `main.rs` — the two are the same operation, just
//! driven by different triggers.
//!
//! Failures are logged and skipped rather than propagated: the journal row
//! for a failed file is left untouched, so it's naturally retried on the
//! next reconcile pass or the next time it's touched — no separate retry/
//! backoff queue, per docs/DESIGN.md. The one exception is a missing/expired
//! session (`DriveError::NotAuthenticated`): every remaining file would fail
//! identically, so [`reconcile`] propagates that instead of logging it once
//! per file — `main.rs` is responsible for reacting to it (see
//! `DaemonError::is_authentication_error`).

use std::path::Path;
use std::time::UNIX_EPOCH;

use protondrive_core::cli::{self, CommandRunner, DriveError};

use crate::config::Config;
use crate::error::DaemonError;
use crate::journal::Journal;

/// Ensures every path segment of `remote_dir` exists as a real folder,
/// creating missing ones one level at a time (`core` has no recursive
/// mkdir). The first segment is always a fixed virtual section (e.g.
/// "/my-files") — assumed to already exist rather than stat'd or created,
/// since stat'ing a bare virtual section is unreliable (some sections
/// respond "not implemented", `/photos` is known to hang — see
/// core/src/cli.rs's METADATA_TIMEOUT comment).
pub fn ensure_remote_dir_chain(
    runner: &dyn CommandRunner,
    remote_dir: &str,
) -> Result<(), DriveError> {
    let segments: Vec<&str> = remote_dir.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 1 {
        return Ok(());
    }

    let mut current = format!("/{}", segments[0]);
    for segment in &segments[1..] {
        let parent = current.clone();
        current.push('/');
        current.push_str(segment);
        match cli::stat_path(runner, &current) {
            Ok(_) => continue,
            Err(DriveError::NotFound(_)) => match cli::create_folder(runner, &parent, segment) {
                Ok(_) | Err(DriveError::AlreadyExists(_)) => continue,
                Err(err) => return Err(err),
            },
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// The remote folder `local_file` should be uploaded into: `config.remote_path`
/// plus whatever subdirectory path `local_file` has relative to
/// `config.local_path`.
fn remote_parent_for(config: &Config, local_file: &Path) -> String {
    let parent_dir = local_file.parent().unwrap_or_else(|| Path::new(""));
    let rel = parent_dir
        .strip_prefix(&config.local_path)
        .unwrap_or(parent_dir);
    let rel_str = rel.to_string_lossy();
    let remote_root = config.remote_path.trim_end_matches('/');
    if rel_str.is_empty() {
        remote_root.to_string()
    } else {
        format!("{remote_root}/{rel_str}")
    }
}

/// The full remote path `local_file` maps to: [`remote_parent_for`] plus its
/// own filename.
fn remote_path_for(config: &Config, local_file: &Path) -> String {
    let parent = remote_parent_for(config, local_file);
    let name = local_file.file_name().unwrap_or_default().to_string_lossy();
    format!("{parent}/{name}")
}

/// Propagates a local rename/move (`from` -> `to`, both inside the watched
/// tree) to Drive, instead of letting it upload as a brand new file and
/// leaving a stale duplicate at the old remote path.
///
/// If `from` has no journal record, nothing was ever successfully uploaded
/// under that path, so there's no remote counterpart to rename — this is
/// just a fresh upload of `to`. Otherwise, the remote file is renamed
/// in place (same directory) or moved (different directory) or both, in
/// that order — `proton-drive`'s `move` doesn't also take a new name.
///
/// If the remote rename/move fails, falls back to a plain upload of `to`
/// rather than propagating the error: a future reconcile walk only ever
/// sees `to` on disk (never `from` again, since it's gone locally), so
/// simply failing here would silently drop the file from sync forever
/// instead of just leaving a stale duplicate at the old remote path.
pub fn handle_rename(
    runner: &dyn CommandRunner,
    journal: &Journal,
    config: &Config,
    from: &Path,
    to: &Path,
) -> Result<(), DaemonError> {
    if journal.get(from)?.is_none() {
        return upload_if_needed(runner, journal, config, to);
    }

    let remote_result = (|| -> Result<(), DriveError> {
        let old_remote_parent = remote_parent_for(config, from);
        let new_remote_parent = remote_parent_for(config, to);
        let old_remote_path = remote_path_for(config, from);
        let new_name = to.file_name().unwrap_or_default().to_string_lossy();

        ensure_remote_dir_chain(runner, &new_remote_parent)?;
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
        Ok(()) => journal.rename(from, to),
        Err(err) => {
            log::warn!(
                "failed to rename/move {} -> {} on Drive ({err}), falling back to a fresh \
                 upload (the old remote copy may now be a stale duplicate)",
                from.display(),
                to.display()
            );
            upload_if_needed(runner, journal, config, to)
        }
    }
}

/// Uploads `local_file` if its mtime/size changed since the last successful
/// upload (per the journal), skipping it otherwise. Not-a-file paths (e.g. a
/// directory create event) are silently ignored.
pub fn upload_if_needed(
    runner: &dyn CommandRunner,
    journal: &Journal,
    config: &Config,
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

    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let size = metadata.len() as i64;

    if !journal.needs_upload(local_file, mtime, size)? {
        return Ok(());
    }

    let remote_parent = remote_parent_for(config, local_file);
    ensure_remote_dir_chain(runner, &remote_parent)?;
    cli::upload(runner, local_file, &remote_parent)?;
    journal.mark_synced(local_file, mtime, size)?;
    Ok(())
}

/// Walks `config.local_path` recursively, uploading every file that needs
/// it. Run once at startup to catch changes made while the daemon wasn't
/// running; the live watcher loop then takes over via the same
/// [`upload_if_needed`].
pub fn reconcile(
    runner: &dyn CommandRunner,
    journal: &Journal,
    config: &Config,
) -> Result<(), DaemonError> {
    walk_and_upload(runner, journal, config, &config.local_path)
}

fn walk_and_upload(
    runner: &dyn CommandRunner,
    journal: &Journal,
    config: &Config,
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
            walk_and_upload(runner, journal, config, &path)?;
        } else if file_type.is_file() {
            if let Err(err) = upload_if_needed(runner, journal, config, &path) {
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

    fn config(local: &Path) -> Config {
        Config {
            local_path: local.to_path_buf(),
            remote_path: "/my-files/Backups".to_string(),
        }
    }

    #[test]
    fn remote_parent_for_a_file_directly_in_the_local_root() {
        let local = Path::new("/home/user/Sync");
        let cfg = config(local);
        let file = local.join("report.pdf");
        assert_eq!(remote_parent_for(&cfg, &file), "/my-files/Backups");
    }

    #[test]
    fn remote_parent_for_a_file_in_a_subdirectory() {
        let local = Path::new("/home/user/Sync");
        let cfg = config(local);
        let file = local.join("sub").join("dir").join("report.pdf");
        assert_eq!(remote_parent_for(&cfg, &file), "/my-files/Backups/sub/dir");
    }

    #[test]
    fn ensure_remote_dir_chain_is_a_noop_for_a_bare_virtual_section() {
        let runner = ScriptedRunner::new(Vec::new());
        ensure_remote_dir_chain(&runner, "/my-files").unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn ensure_remote_dir_chain_creates_a_missing_folder() {
        let runner = ScriptedRunner::new(vec![failure(r#"Path not found"#), success(NODE_JSON)]);
        ensure_remote_dir_chain(&runner, "/my-files/Backups").unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            vec!["filesystem", "info", "-j", "/my-files/Backups"]
        );
        assert_eq!(
            calls[1],
            vec!["filesystem", "create-folder", "-j", "/my-files", "Backups"]
        );
    }

    #[test]
    fn ensure_remote_dir_chain_treats_a_racing_create_as_success() {
        // stat says missing, but by the time create-folder runs something
        // else (another sync cycle, a concurrent process) already made it —
        // that's still the outcome this function exists to guarantee, so it
        // shouldn't surface as an error.
        let runner = ScriptedRunner::new(vec![
            failure(r#"Path not found"#),
            failure("Un fichier ou un dossier portant ce nom existe déjà."),
        ]);
        ensure_remote_dir_chain(&runner, "/my-files/Backups").unwrap();
    }

    #[test]
    fn upload_if_needed_skips_a_file_already_synced_at_the_same_mtime_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"hello").unwrap();

        let db_path = dir.path().join("journal.sqlite3");
        let journal = Journal::open(&db_path).unwrap();
        let metadata = std::fs::metadata(&file).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        journal
            .mark_synced(&file, mtime, metadata.len() as i64)
            .unwrap();

        let cfg = config(dir.path());
        // Empty response queue: any CLI call would panic on the missing
        // scripted response, which is exactly how this test proves nothing
        // was called.
        let runner = ScriptedRunner::new(Vec::new());
        upload_if_needed(&runner, &journal, &cfg, &file).unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn upload_if_needed_uploads_an_unknown_file_and_records_it_in_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"hello").unwrap();

        let db_path = dir.path().join("journal.sqlite3");
        let journal = Journal::open(&db_path).unwrap();
        let cfg = config(dir.path());

        // ensure_remote_dir_chain's single "info" call on /my-files/Backups
        // succeeds (already exists), then the upload itself.
        let runner = ScriptedRunner::new(vec![success(NODE_JSON), success(TRANSFER_OK)]);
        upload_if_needed(&runner, &journal, &cfg, &file).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1][0..3], ["filesystem", "upload", "-j"]);

        let metadata = std::fs::metadata(&file).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(!journal
            .needs_upload(&file, mtime, metadata.len() as i64)
            .unwrap());
    }

    /// A single-node JSON response shaped like `rename_path`'s real output
    /// (confirmed live — see core/src/cli.rs's `rename_path`).
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

    fn seed_journal_record(journal: &Journal, path: &Path) {
        journal.mark_synced(path, 100, 5).unwrap();
    }

    #[test]
    fn handle_rename_renames_in_place_for_a_same_directory_rename() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(&dir.path().join("journal.sqlite3")).unwrap();
        let cfg = config(dir.path());
        let from = dir.path().join("old-name.pdf");
        let to = dir.path().join("new-name.pdf");
        seed_journal_record(&journal, &from);

        // ensure_remote_dir_chain's single "info" on /my-files/Backups
        // (already exists), then the rename itself.
        let runner = ScriptedRunner::new(vec![success(NODE_JSON), success(RENAMED_NODE_JSON)]);
        handle_rename(&runner, &journal, &cfg, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1],
            vec![
                "filesystem",
                "rename",
                "-j",
                "/my-files/Backups/old-name.pdf",
                "new-name.pdf",
            ]
        );
        assert!(journal.get(&from).unwrap().is_none());
        assert!(journal.get(&to).unwrap().is_some());
    }

    #[test]
    fn handle_rename_moves_for_a_cross_directory_move_with_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let journal = Journal::open(&dir.path().join("journal.sqlite3")).unwrap();
        let cfg = config(dir.path());
        let from = dir.path().join("report.pdf");
        let to = dir.path().join("sub").join("report.pdf");
        seed_journal_record(&journal, &from);

        // ensure_remote_dir_chain walks two segments to reach
        // /my-files/Backups/sub ("Backups", then "sub"), both already
        // existing ("info" x2), then the move itself.
        let runner = ScriptedRunner::new(vec![
            success(NODE_JSON),
            success(NODE_JSON),
            success(MOVE_OK),
        ]);
        handle_rename(&runner, &journal, &cfg, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[2],
            vec![
                "filesystem",
                "move",
                "-j",
                "/my-files/Backups/report.pdf",
                "/my-files/Backups/sub",
            ]
        );
        assert!(journal.get(&to).unwrap().is_some());
    }

    #[test]
    fn handle_rename_moves_then_renames_for_a_cross_directory_move_with_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let journal = Journal::open(&dir.path().join("journal.sqlite3")).unwrap();
        let cfg = config(dir.path());
        let from = dir.path().join("old-name.pdf");
        let to = dir.path().join("sub").join("new-name.pdf");
        seed_journal_record(&journal, &from);

        let runner = ScriptedRunner::new(vec![
            success(NODE_JSON),
            success(NODE_JSON),
            success(MOVE_OK),
            success(RENAMED_NODE_JSON),
        ]);
        handle_rename(&runner, &journal, &cfg, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[2],
            vec![
                "filesystem",
                "move",
                "-j",
                "/my-files/Backups/old-name.pdf",
                "/my-files/Backups/sub",
            ]
        );
        assert_eq!(
            calls[3],
            vec![
                "filesystem",
                "rename",
                "-j",
                "/my-files/Backups/sub/old-name.pdf",
                "new-name.pdf",
            ]
        );
    }

    #[test]
    fn handle_rename_falls_back_to_upload_when_the_old_path_has_no_journal_record() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("new-name.pdf");
        std::fs::write(&to, b"hello").unwrap();
        let journal = Journal::open(&dir.path().join("journal.sqlite3")).unwrap();
        let cfg = config(dir.path());
        let from = dir.path().join("old-name.pdf"); // never uploaded

        let runner = ScriptedRunner::new(vec![success(NODE_JSON), success(TRANSFER_OK)]);
        handle_rename(&runner, &journal, &cfg, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(calls[1][0..3], ["filesystem", "upload", "-j"]);
    }

    #[test]
    fn handle_rename_falls_back_to_upload_when_the_remote_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("new-name.pdf");
        std::fs::write(&to, b"hello").unwrap();
        let journal = Journal::open(&dir.path().join("journal.sqlite3")).unwrap();
        let cfg = config(dir.path());
        let from = dir.path().join("old-name.pdf");
        seed_journal_record(&journal, &from);

        // ensure_remote_dir_chain's "info" for the (failed) rename attempt,
        // the failing rename itself, then the fallback upload's own
        // ensure_remote_dir_chain "info" and the upload.
        let runner = ScriptedRunner::new(vec![
            success(NODE_JSON),
            failure("internal server error, please retry"),
            success(NODE_JSON),
            success(TRANSFER_OK),
        ]);
        handle_rename(&runner, &journal, &cfg, &from, &to).unwrap();

        let calls = runner.calls.borrow();
        assert_eq!(
            calls[1],
            vec![
                "filesystem",
                "rename",
                "-j",
                "/my-files/Backups/old-name.pdf",
                "new-name.pdf",
            ]
        );
        assert_eq!(calls[3][0..3], ["filesystem", "upload", "-j"]);
        // The fallback upload should have recorded `to` as synced.
        let metadata = std::fs::metadata(&to).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(!journal
            .needs_upload(&to, mtime, metadata.len() as i64)
            .unwrap());
    }

    #[test]
    fn reconcile_stops_the_whole_walk_on_the_first_authentication_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.pdf"), b"hello").unwrap();
        std::fs::write(dir.path().join("b.pdf"), b"world").unwrap();

        let db_path = dir.path().join("journal.sqlite3");
        let journal = Journal::open(&db_path).unwrap();
        let cfg = config(dir.path());

        let runner = ScriptedRunner::new(vec![failure("You need to login first")]);
        let err = reconcile(&runner, &journal, &cfg).unwrap_err();
        assert!(err.is_authentication_error());
        // Only the first file's ensure_remote_dir_chain "info" call
        // happened — the walk stopped immediately rather than also trying
        // the second file, which would fail identically.
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}
