//! Thin wrapper around the `proton-drive` CLI.
//!
//! All calls go through the [`CommandRunner`] trait rather than
//! `std::process::Command` directly, so unit tests can substitute a mock that
//! returns canned JSON instead of spawning the real binary — the real CLI
//! needs a live, authenticated Proton Drive session, which unit tests (and CI)
//! don't have.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::entry::{ListItem, NodeEntry, TransferSummary, TrashOutcome};

/// `filesystem list`/`info`/`create-folder`/`trash` are plain metadata API
/// calls — a healthy CLI answers in well under this. Deliberately short so a
/// hang (e.g. the CLI blocking indefinitely on an unsupported virtual path
/// like `/photos`'s `filesystem info`, rather than erroring out — see the
/// "Not implemented"-vs-hang split observed in the wild) can't tie up a KIO
/// worker slot for long.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);

/// `filesystem download`/`upload` transfer actual file data, so their
/// duration legitimately scales with file size — this is a generous safety
/// net against a truly stuck CLI, not a realistic transfer-time budget.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DriveError {
    #[error("path not found: {0}")]
    NotFound(String),
    #[error("proton-drive reported an error: {0}")]
    Cli(String),
    #[error("could not parse proton-drive output: {0}")]
    Parse(String),
    #[error("could not launch proton-drive: {0}")]
    Spawn(String),
    #[error("proton-drive did not respond within {0:?}")]
    Timeout(Duration),
}

impl From<serde_json::Error> for DriveError {
    fn from(err: serde_json::Error) -> Self {
        DriveError::Parse(err.to_string())
    }
}

/// Raw result of running the CLI, before JSON parsing / error interpretation.
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Abstraction over "run the `proton-drive` CLI with these arguments",
/// injectable so tests never need a real installation or session.
pub trait CommandRunner {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<CommandOutput, DriveError>;
}

/// Real implementation: spawns the `proton-drive` binary from `$PATH`.
#[derive(Debug, Default, Clone)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<CommandOutput, DriveError> {
        let child = std::process::Command::new("proton-drive")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| DriveError::Spawn(err.to_string()))?;
        let pid = child.id();

        // `Child::wait_with_output` (needed to drain stdout/stderr without
        // risking the classic full-pipe deadlock) has no timeout variant, so
        // it runs on a helper thread; a channel gives us the `recv_timeout`
        // that's actually missing here. On timeout the child is killed by
        // pid — `child` itself was moved into the thread, so this is the
        // only handle left to reach it — and the thread's own `send` then
        // just fails silently since `rx` is already dropped by then.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(output)) => Ok(CommandOutput {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                success: output.status.success(),
            }),
            Ok(Err(err)) => Err(DriveError::Spawn(err.to_string())),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .status();
                Err(DriveError::Timeout(timeout))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(DriveError::Spawn(
                "proton-drive's output thread vanished without a result".to_string(),
            )),
        }
    }
}

/// Even with `-j/--json`, the CLI prints a plain-text message to stderr (not
/// JSON) and exits non-zero on error — this must be checked before attempting
/// to parse stdout as JSON.
fn ensure_success(path: &str, out: &CommandOutput) -> Result<(), DriveError> {
    if out.success {
        return Ok(());
    }
    let message = out.stderr.trim();
    let lower = message.to_lowercase();
    if lower.contains("not supported") || lower.contains("not found") {
        Err(DriveError::NotFound(path.to_string()))
    } else if message.is_empty() {
        Err(DriveError::Cli(format!(
            "proton-drive exited with an error for {path}"
        )))
    } else {
        Err(DriveError::Cli(message.to_string()))
    }
}

pub fn list_dir(runner: &dyn CommandRunner, path: &str) -> Result<Vec<ListItem>, DriveError> {
    let out = runner.run(&["filesystem", "list", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    Ok(serde_json::from_str(&out.stdout)?)
}

pub fn stat_path(runner: &dyn CommandRunner, path: &str) -> Result<NodeEntry, DriveError> {
    let out = runner.run(&["filesystem", "info", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    Ok(serde_json::from_str(&out.stdout)?)
}

pub fn create_folder(
    runner: &dyn CommandRunner,
    parent_path: &str,
    name: &str,
) -> Result<NodeEntry, DriveError> {
    let out = runner.run(
        &["filesystem", "create-folder", "-j", parent_path, name],
        METADATA_TIMEOUT,
    )?;
    ensure_success(parent_path, &out)?;
    Ok(serde_json::from_str(&out.stdout)?)
}

fn ensure_no_failures(context: &str, summary: &TransferSummary) -> Result<(), DriveError> {
    if summary.failed_items > 0 {
        return Err(DriveError::Cli(format!(
            "{context} reported {} failed item(s)",
            summary.failed_items
        )));
    }
    Ok(())
}

/// Downloads `remote_path` into `local_folder` (which must already exist).
/// Always forces the `replace` file-conflict strategy: the worker downloads
/// into a fresh temporary directory it controls, so there is never a
/// legitimate local file to preserve, and the CLI would otherwise block
/// waiting for interactive input the worker process has no way to provide.
pub fn download(
    runner: &dyn CommandRunner,
    remote_path: &str,
    local_folder: &Path,
) -> Result<TransferSummary, DriveError> {
    let local = local_folder.to_string_lossy();
    let out = runner.run(
        &[
            "filesystem",
            "download",
            "-j",
            "-f",
            "replace",
            remote_path,
            &local,
        ],
        TRANSFER_TIMEOUT,
    )?;
    ensure_success(remote_path, &out)?;
    let summary: TransferSummary = serde_json::from_str(&out.stdout)?;
    ensure_no_failures(&format!("download of {remote_path}"), &summary)?;
    Ok(summary)
}

/// Uploads `local_path` into `parent_path`. Forces `replace` for the same
/// reason as [`download`] — an interactive prompt would hang the worker.
pub fn upload(
    runner: &dyn CommandRunner,
    local_path: &Path,
    parent_path: &str,
) -> Result<TransferSummary, DriveError> {
    let local = local_path.to_string_lossy();
    let out = runner.run(
        &[
            "filesystem",
            "upload",
            "-j",
            "-f",
            "replace",
            &local,
            parent_path,
        ],
        TRANSFER_TIMEOUT,
    )?;
    ensure_success(parent_path, &out)?;
    let summary: TransferSummary = serde_json::from_str(&out.stdout)?;
    ensure_no_failures(&format!("upload to {parent_path}"), &summary)?;
    Ok(summary)
}

/// Moves `path` to Proton Drive's trash (soft delete — matches KIO `del`,
/// see core/README notes on why this project doesn't expose a permanent
/// delete through Dolphin in v1).
pub fn trash_path(runner: &dyn CommandRunner, path: &str) -> Result<(), DriveError> {
    let out = runner.run(&["filesystem", "trash", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    let outcomes: Vec<TrashOutcome> = serde_json::from_str(&out.stdout)?;
    if let Some(failed) = outcomes.iter().find(|o| !o.ok) {
        return Err(DriveError::Cli(format!("failed to trash {}", failed.uid)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Records the args it was called with and returns a pre-set response —
    /// sanitized fixture data below, not real Proton Drive output.
    struct MockRunner {
        response: CommandOutput,
        last_args: RefCell<Vec<String>>,
    }

    impl MockRunner {
        fn success(stdout: &str) -> Self {
            Self {
                response: CommandOutput {
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                    success: true,
                },
                last_args: RefCell::new(Vec::new()),
            }
        }

        fn failure(stderr: &str) -> Self {
            Self {
                response: CommandOutput {
                    stdout: String::new(),
                    stderr: stderr.to_string(),
                    success: false,
                },
                last_args: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
            *self.last_args.borrow_mut() = args.iter().map(|s| s.to_string()).collect();
            Ok(CommandOutput {
                stdout: self.response.stdout.clone(),
                stderr: self.response.stderr.clone(),
                success: self.response.success,
            })
        }
    }

    const ROOT_LISTING: &str = r#"[
        {"path":"/my-files"},
        {"path":"/devices"},
        {"path":"/trash"}
    ]"#;

    const FOLDER_LISTING: &str = r#"[
        {
            "uid":"uid-folder-1",
            "name":{"ok":true,"value":"Photos"},
            "type":"folder",
            "isShared":false,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z",
            "folder":{"isImported":false}
        },
        {
            "uid":"uid-file-1",
            "name":{"ok":true,"value":"report.pdf"},
            "type":"file",
            "mediaType":"application/pdf",
            "totalStorageSize":12345,
            "isShared":false,
            "creationTime":"2026-01-02T00:00:00.000Z",
            "modificationTime":"2026-01-02T00:00:00.000Z"
        },
        {
            "uid":"uid-file-undecryptable-name",
            "name":{"ok":false},
            "type":"file",
            "totalStorageSize":10,
            "isShared":false,
            "creationTime":"2026-01-03T00:00:00.000Z",
            "modificationTime":"2026-01-03T00:00:00.000Z"
        }
    ]"#;

    const TRANSFER_SUMMARY_OK: &str = r#"{"transferredItems":1,"transferredBytes":26,"skippedItems":0,"failedItems":0,"failures":[]}"#;

    const TRANSFER_SUMMARY_FAILED: &str = r#"{"transferredItems":0,"transferredBytes":0,"skippedItems":0,"failedItems":1,"failures":[{"reason":"network error"}]}"#;

    const TRASH_OK: &str = r#"[{"uid":"uid-folder-1","ok":true}]"#;
    const TRASH_PARTIAL_FAILURE: &str = r#"[{"uid":"uid-a","ok":true},{"uid":"uid-b","ok":false}]"#;

    #[test]
    fn list_dir_parses_virtual_root_sections() {
        let runner = MockRunner::success(ROOT_LISTING);
        let items = list_dir(&runner, "/").unwrap();
        assert_eq!(items.len(), 3);
        match &items[0] {
            ListItem::Section(section) => assert_eq!(section.display_name(), "my-files"),
            ListItem::Node(_) => panic!("expected a virtual section, got a node"),
        }
    }

    #[test]
    fn list_dir_parses_real_nodes_and_falls_back_to_uid_for_undecryptable_names() {
        let runner = MockRunner::success(FOLDER_LISTING);
        let items = list_dir(&runner, "/my-files").unwrap();
        assert_eq!(items.len(), 3);

        let folder = match &items[0] {
            ListItem::Node(node) => node,
            ListItem::Section(_) => panic!("expected a node"),
        };
        assert!(folder.is_folder());
        assert_eq!(folder.display_name(), "Photos");

        let file = match &items[1] {
            ListItem::Node(node) => node,
            ListItem::Section(_) => panic!("expected a node"),
        };
        assert!(!file.is_folder());
        assert_eq!(file.display_name(), "report.pdf");
        assert_eq!(file.total_storage_size, Some(12345));
        assert_eq!(file.media_type.as_deref(), Some("application/pdf"));

        let undecryptable = match &items[2] {
            ListItem::Node(node) => node,
            ListItem::Section(_) => panic!("expected a node"),
        };
        assert_eq!(undecryptable.display_name(), "uid-file-undecryptable-name");
    }

    #[test]
    fn list_dir_passes_json_flag_and_path_through_to_the_runner() {
        let runner = MockRunner::success(ROOT_LISTING);
        list_dir(&runner, "/my-files").unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec!["filesystem", "list", "-j", "/my-files"]
        );
    }

    #[test]
    fn list_dir_maps_cli_failure_to_not_found() {
        let runner = MockRunner::failure(r#"Path "/nope" not supported"#);
        let err = list_dir(&runner, "/nope").unwrap_err();
        assert_eq!(err, DriveError::NotFound("/nope".to_string()));
    }

    #[test]
    fn list_dir_maps_unrecognized_cli_failure_to_generic_cli_error() {
        let runner = MockRunner::failure("session expired, please log in again");
        let err = list_dir(&runner, "/my-files").unwrap_err();
        assert_eq!(
            err,
            DriveError::Cli("session expired, please log in again".to_string())
        );
    }

    #[test]
    fn download_forces_replace_conflict_strategy() {
        let runner = MockRunner::success(TRANSFER_SUMMARY_OK);
        let dest = Path::new("/tmp/kio-protondrive-test-dest");
        download(&runner, "/my-files/report.pdf", dest).unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec![
                "filesystem",
                "download",
                "-j",
                "-f",
                "replace",
                "/my-files/report.pdf",
                "/tmp/kio-protondrive-test-dest",
            ]
        );
    }

    #[test]
    fn download_surfaces_partial_failures_as_an_error() {
        let runner = MockRunner::success(TRANSFER_SUMMARY_FAILED);
        let err = download(&runner, "/my-files/report.pdf", Path::new("/tmp/x")).unwrap_err();
        assert!(matches!(err, DriveError::Cli(_)));
    }

    #[test]
    fn upload_forces_replace_conflict_strategy() {
        let runner = MockRunner::success(TRANSFER_SUMMARY_OK);
        upload(&runner, Path::new("/tmp/report.pdf"), "/my-files").unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec![
                "filesystem",
                "upload",
                "-j",
                "-f",
                "replace",
                "/tmp/report.pdf",
                "/my-files",
            ]
        );
    }

    #[test]
    fn trash_path_succeeds_when_all_outcomes_are_ok() {
        let runner = MockRunner::success(TRASH_OK);
        trash_path(&runner, "/my-files/Photos").unwrap();
    }

    #[test]
    fn trash_path_errors_when_any_outcome_failed() {
        let runner = MockRunner::success(TRASH_PARTIAL_FAILURE);
        let err = trash_path(&runner, "/my-files").unwrap_err();
        assert!(matches!(err, DriveError::Cli(_)));
    }

    #[test]
    fn create_folder_parses_the_created_node() {
        const CREATED: &str = r#"{
            "uid":"uid-new-folder",
            "name":{"ok":true,"value":"New Folder"},
            "type":"folder",
            "isShared":false,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z"
        }"#;
        let runner = MockRunner::success(CREATED);
        let node = create_folder(&runner, "/my-files", "New Folder").unwrap();
        assert_eq!(node.display_name(), "New Folder");
        assert!(node.is_folder());
    }

    #[test]
    fn stat_path_parses_a_single_node() {
        const INFO: &str = r#"{
            "uid":"uid-root",
            "name":{"ok":true,"value":"root"},
            "type":"folder",
            "isShared":true,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z"
        }"#;
        let runner = MockRunner::success(INFO);
        let node = stat_path(&runner, "/my-files").unwrap();
        assert_eq!(node.uid, "uid-root");
    }
}
