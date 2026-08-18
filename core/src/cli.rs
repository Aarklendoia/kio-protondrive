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
    #[error("not logged in to Proton Drive — run \"proton-drive auth login\"")]
    NotAuthenticated,
    #[error("a file or folder with this name already exists: {0}")]
    AlreadyExists(String),
    #[error("proton-drive reported an error: {0}")]
    Cli(String),
    #[error("could not parse proton-drive output: {0}")]
    Parse(String),
    #[error("could not launch proton-drive: {0}")]
    Spawn(String),
    #[error("proton-drive did not respond within {0:?}")]
    Timeout(Duration),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("cache database error: {0}")]
    Sqlite(String),
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
    if lower.contains("login")
        || lower.contains("log in")
        || lower.contains("not authenticated")
        || lower.contains("invalid access token")
        || lower.contains("invalid refresh token")
    {
        Err(DriveError::NotAuthenticated)
    } else if lower.contains("not supported") || lower.contains("not found") {
        Err(DriveError::NotFound(path.to_string()))
    } else if lower.contains("already exist") || lower.contains("existe déjà") {
        // Confirmed live that this specific message comes back in French
        // ("Un fichier ou un dossier portant ce nom existe déjà.") even with
        // LC_ALL=C / LANG=en_US.UTF-8 forced — so it's not driven by the
        // usual locale env vars this process sees, unlike "You need to login
        // first" and other messages, which do stay in English regardless.
        // The English phrasing here is a best-effort guess, not something
        // observed directly.
        Err(DriveError::AlreadyExists(path.to_string()))
    } else if message.is_empty() {
        Err(DriveError::Cli(format!(
            "proton-drive exited with an error for {path}{}",
            stdout_context(out)
        )))
    } else {
        Err(DriveError::Cli(format!("{message}{}", stdout_context(out))))
    }
}

/// Appends the CLI's stdout to an error message when it might hold extra
/// diagnostic context stderr didn't (e.g. a stack trace, or the "===...==="
/// banner seen in #38) — stderr alone is the CLI's *intended* error channel
/// and is usually sufficient, but this is a cheap way to capture more detail
/// if/when a case like #38 recurs, without having to reproduce it first.
fn stdout_context(out: &CommandOutput) -> String {
    let stdout = out.stdout.trim();
    if stdout.is_empty() {
        String::new()
    } else {
        format!(" (stdout: {stdout})")
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

/// Ensures every path segment of `remote_dir` exists as a real folder,
/// creating missing ones one level at a time (no recursive mkdir on the
/// CLI side). The first segment is always a fixed virtual section (e.g.
/// "/my-files") — assumed to already exist rather than stat'd or created,
/// since stat'ing a bare virtual section is unreliable (some sections
/// respond "not implemented", `/photos` is known to hang — see
/// METADATA_TIMEOUT's comment above). Shared by the daemon (building the
/// remote folder chain for a local subdirectory) and the setup wizard
/// (validating/creating the configured remote folder).
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
        match stat_path(runner, &current) {
            Ok(_) => continue,
            Err(DriveError::NotFound(_)) => match create_folder(runner, &parent, segment) {
                Ok(_) | Err(DriveError::AlreadyExists(_)) => continue,
                Err(err) => return Err(err),
            },
            Err(err) => return Err(err),
        }
    }
    Ok(())
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
/// Always forces the `remove` file-conflict strategy (`download`'s own
/// equivalent of `upload`'s `replace` — confirmed via `filesystem download
/// --help`, whose accepted `-f` values are `rename`/`remove`/`skip`, not
/// `replace`): the worker downloads into a fresh temporary directory it
/// controls, so there is never a legitimate local file to preserve, and the
/// CLI would otherwise block waiting for interactive input the worker
/// process has no way to provide.
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
            "remove",
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

/// `filesystem upload`'s `localPath...` argument is glob-matched by the CLI
/// (it's how it supports uploading several files at once, e.g. `*.pdf`) —
/// but every call site here always means one literal, already-resolved
/// path, never a pattern. `[`, `]`, `{` and `}` are the metacharacters that
/// actually trip this up in practice: confirmed live that e.g. a file named
/// "report [2026].pdf" fails with the CLI's own "No paths matched: ..."
/// error unless those are backslash-escaped first. `*`, `?` and `!` were
/// checked too and do *not* need escaping — escaping them when there's
/// nothing to escape instead makes the CLI silently skip the file, so this
/// deliberately only touches the characters confirmed to need it.
fn escape_glob_metacharacters(path: &str) -> String {
    path.replace('[', "\\[")
        .replace(']', "\\]")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

/// Uploads `local_path` into `parent_path`. Forces `replace` for the same
/// reason as [`download`] — an interactive prompt would hang the worker.
pub fn upload(
    runner: &dyn CommandRunner,
    local_path: &Path,
    parent_path: &str,
) -> Result<TransferSummary, DriveError> {
    let local = escape_glob_metacharacters(&local_path.to_string_lossy());
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

/// Renames the node at `path` in place to `new_name`, without moving it to a
/// different folder (that's [`move_path`]). Confirmed live that the response
/// is a single node object, same shape as `create_folder`'s.
pub fn rename_path(
    runner: &dyn CommandRunner,
    path: &str,
    new_name: &str,
) -> Result<NodeEntry, DriveError> {
    let out = runner.run(
        &["filesystem", "rename", "-j", path, new_name],
        METADATA_TIMEOUT,
    )?;
    ensure_success(path, &out)?;
    Ok(serde_json::from_str(&out.stdout)?)
}

/// Moves `source_path` into `target_parent_path`, keeping its name (use
/// [`rename_path`] to also change the name). Confirmed live that the
/// response is the same `[{uid, ok}, ...]` shape as `trash_path`'s.
pub fn move_path(
    runner: &dyn CommandRunner,
    source_path: &str,
    target_parent_path: &str,
) -> Result<(), DriveError> {
    let out = runner.run(
        &["filesystem", "move", "-j", source_path, target_parent_path],
        METADATA_TIMEOUT,
    )?;
    ensure_success(source_path, &out)?;
    let outcomes: Vec<TrashOutcome> = serde_json::from_str(&out.stdout)?;
    if let Some(failed) = outcomes.iter().find(|o| !o.ok) {
        return Err(DriveError::Cli(format!("failed to move {}", failed.uid)));
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

    /// Like `MockRunner`, but returns a different pre-set response per call
    /// (in order) instead of the same one every time — needed for functions
    /// like `ensure_remote_dir_chain` that make more than one CLI call per
    /// invocation and expect different outcomes from each.
    struct ScriptedRunner {
        responses: RefCell<std::collections::VecDeque<CommandOutput>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedRunner {
        fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
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

    fn success(stdout: &str) -> CommandOutput {
        CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            success: true,
        }
    }

    fn failure(stderr: &str) -> CommandOutput {
        CommandOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
            success: false,
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
    fn list_dir_appends_stdout_to_an_unrecognized_cli_failure_for_diagnostics() {
        let runner = MockRunner {
            response: CommandOutput {
                stdout: "===============================================".to_string(),
                stderr: "internal server error, please retry".to_string(),
                success: false,
            },
            last_args: RefCell::new(Vec::new()),
        };
        let err = list_dir(&runner, "/my-files").unwrap_err();
        assert_eq!(
            err,
            DriveError::Cli(
                "internal server error, please retry (stdout: \
                 ===============================================)"
                    .to_string()
            )
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
        let runner = MockRunner::failure("internal server error, please retry");
        let err = list_dir(&runner, "/my-files").unwrap_err();
        assert_eq!(
            err,
            DriveError::Cli("internal server error, please retry".to_string())
        );
    }

    /// Exact message confirmed against the real CLI (`proton-drive filesystem
    /// list -j /` with no session): stderr "You need to login first", exit
    /// code 1, empty stdout.
    #[test]
    fn list_dir_maps_login_required_to_not_authenticated() {
        let runner = MockRunner::failure("You need to login first");
        let err = list_dir(&runner, "/my-files").unwrap_err();
        assert_eq!(err, DriveError::NotAuthenticated);
    }

    #[test]
    fn list_dir_maps_session_expiry_to_not_authenticated() {
        let runner = MockRunner::failure("session expired, please log in again");
        let err = list_dir(&runner, "/my-files").unwrap_err();
        assert_eq!(err, DriveError::NotAuthenticated);
    }

    #[test]
    fn download_forces_remove_conflict_strategy() {
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
                "remove",
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
    fn upload_escapes_glob_metacharacters_in_the_local_path() {
        let runner = MockRunner::success(TRANSFER_SUMMARY_OK);
        upload(
            &runner,
            Path::new("/tmp/report [2026] {final}.pdf"),
            "/my-files",
        )
        .unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec![
                "filesystem",
                "upload",
                "-j",
                "-f",
                "replace",
                "/tmp/report \\[2026\\] \\{final\\}.pdf",
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
    fn rename_path_parses_the_renamed_node() {
        const RENAMED: &str = r#"{
            "uid":"uid-file",
            "name":{"ok":true,"value":"new-name.txt"},
            "type":"file",
            "isShared":false,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z"
        }"#;
        let runner = MockRunner::success(RENAMED);
        let node = rename_path(&runner, "/my-files/old-name.txt", "new-name.txt").unwrap();
        assert_eq!(node.display_name(), "new-name.txt");
        assert_eq!(
            *runner.last_args.borrow(),
            vec![
                "filesystem",
                "rename",
                "-j",
                "/my-files/old-name.txt",
                "new-name.txt",
            ]
        );
    }

    #[test]
    fn move_path_succeeds_when_all_outcomes_are_ok() {
        let runner = MockRunner::success(TRASH_OK);
        move_path(&runner, "/my-files/a.txt", "/my-files/Sub").unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec![
                "filesystem",
                "move",
                "-j",
                "/my-files/a.txt",
                "/my-files/Sub",
            ]
        );
    }

    #[test]
    fn move_path_errors_when_any_outcome_failed() {
        let runner = MockRunner::success(TRASH_PARTIAL_FAILURE);
        let err = move_path(&runner, "/my-files/a.txt", "/my-files/Sub").unwrap_err();
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
    fn create_folder_maps_already_exists_in_english() {
        let runner = MockRunner::failure("A file or folder with this name already exists.");
        let err = create_folder(&runner, "/my-files", "Backups").unwrap_err();
        assert!(matches!(err, DriveError::AlreadyExists(_)));
    }

    #[test]
    fn create_folder_maps_already_exists_in_french() {
        let runner = MockRunner::failure("Un fichier ou un dossier portant ce nom existe déjà.");
        let err = create_folder(&runner, "/my-files", "Backups").unwrap_err();
        assert!(matches!(err, DriveError::AlreadyExists(_)));
    }

    const CREATED_FOLDER_NODE: &str = r#"{
        "uid":"uid-new-folder",
        "name":{"ok":true,"value":"Backups"},
        "type":"folder",
        "isShared":false,
        "creationTime":"2026-01-01T00:00:00.000Z",
        "modificationTime":"2026-01-01T00:00:00.000Z"
    }"#;

    #[test]
    fn ensure_remote_dir_chain_is_a_noop_for_a_bare_virtual_section() {
        let runner = ScriptedRunner::new(Vec::new());
        ensure_remote_dir_chain(&runner, "/my-files").unwrap();
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn ensure_remote_dir_chain_creates_a_missing_folder() {
        let runner = ScriptedRunner::new(vec![
            failure("Path not found"),
            success(CREATED_FOLDER_NODE),
        ]);
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
            failure("Path not found"),
            failure("Un fichier ou un dossier portant ce nom existe déjà."),
        ]);
        ensure_remote_dir_chain(&runner, "/my-files/Backups").unwrap();
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
