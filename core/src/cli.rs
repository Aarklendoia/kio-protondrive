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

use crate::entry::{ListItem, NodeEntry, PublicLink, SharingStatus, TransferSummary, TrashOutcome};

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

/// Up to 4 protondrive:// KIO worker instances (`maxInstances` in
/// worker/protondrive.json) plus the sync daemon can each independently
/// shell out to `proton-drive` at once — confirmed live (#38) that this
/// occasionally collides on the CLI's *own* local SQLite cache database
/// ("database is locked" / `SQLITE_BUSY_RECOVERY` while it opens that file,
/// before attempting the actual Drive operation — so nothing has happened
/// yet on our side, safe to retry), especially now that browsing `/photos`
/// can fire many concurrent `photo download` calls in a burst (one per
/// visible thumbnail, see `crate::photos`). #38 already observed live that
/// retrying the exact same command shortly after succeeds.
pub(crate) const LOCK_CONTENTION_RETRIES: u32 = 3;
pub(crate) const LOCK_CONTENTION_RETRY_DELAY: Duration = Duration::from_millis(300);

pub(crate) fn is_transient_lock_contention(out: &CommandOutput) -> bool {
    !out.success
        && (out.stdout.contains("database is locked")
            || out.stdout.contains("SQLITE_BUSY")
            || out.stderr.contains("database is locked")
            || out.stderr.contains("SQLITE_BUSY"))
}

impl RealCommandRunner {
    fn run_once(&self, args: &[&str], timeout: Duration) -> Result<CommandOutput, DriveError> {
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

impl CommandRunner for RealCommandRunner {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<CommandOutput, DriveError> {
        let mut out = self.run_once(args, timeout)?;
        for attempt in 1..LOCK_CONTENTION_RETRIES {
            if !is_transient_lock_contention(&out) {
                break;
            }
            thread::sleep(LOCK_CONTENTION_RETRY_DELAY * attempt);
            out = self.run_once(args, timeout)?;
        }
        Ok(out)
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

/// Argument-building half of [`download`] — split out so
/// `crate::transfer::TransferHandle` can rebuild the exact same argument
/// list for a lock-contention retry re-spawn (see #38) without going through
/// [`CommandRunner`], which the cancellable worker path doesn't use.
pub(crate) fn download_args(remote_path: &str, local_folder: &Path) -> Vec<String> {
    vec![
        "filesystem".to_string(),
        "download".to_string(),
        "-j".to_string(),
        "-f".to_string(),
        "remove".to_string(),
        remote_path.to_string(),
        local_folder.to_string_lossy().into_owned(),
    ]
}

/// Result-checking half of [`download`] — split out so the cancellable
/// worker path (`crate::transfer`) can apply the exact same success/failure
/// handling to a [`CommandOutput`] obtained by polling a [`TransferHandle`]
/// instead of a single blocking [`CommandRunner::run`] call.
///
/// [`TransferHandle`]: crate::transfer::TransferHandle
pub(crate) fn finish_download(
    remote_path: &str,
    out: CommandOutput,
) -> Result<TransferSummary, DriveError> {
    ensure_success(remote_path, &out)?;
    let summary: TransferSummary = serde_json::from_str(&out.stdout)?;
    ensure_no_failures(&format!("download of {remote_path}"), &summary)?;
    Ok(summary)
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
    let args = download_args(remote_path, local_folder);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = runner.run(&arg_refs, TRANSFER_TIMEOUT)?;
    finish_download(remote_path, out)
}

/// Lists every photo in the account (`photo timeline`), newest first — a
/// completely separate command family from `filesystem`, addressing photos
/// by `nodeUid` rather than by Drive path (see #18: `/photos` genuinely
/// isn't supported through `filesystem list`/`info`, but CLI 0.7.0+ added
/// this dedicated family instead). `-d`/`--load-details` is required to get
/// full node metadata (name, size, media type, ...) — without it, each
/// entry is just `{nodeUid, captureTime, tags}`, useless for a file listing.
///
/// There's no pagination or per-item detail fetch (checked `--help`: `-d`
/// is the only flag `photo timeline` takes), so this is a single call for
/// the *entire* library — confirmed live at ~80s for a ~12k-photo account.
/// TRANSFER_TIMEOUT rather than METADATA_TIMEOUT for that reason: this is a
/// bulk operation, not the quick per-path metadata call every other use of
/// METADATA_TIMEOUT is. `crate::bridge`'s process-lifetime cache is what
/// keeps this from being called on every `stat`/`get` (i.e. every
/// thumbnail).
pub fn photo_timeline(runner: &dyn CommandRunner) -> Result<Vec<NodeEntry>, DriveError> {
    let out = runner.run(&["photo", "timeline", "-j", "-d"], TRANSFER_TIMEOUT)?;
    ensure_success("/photos", &out)?;
    Ok(serde_json::from_str(&out.stdout)?)
}

/// Downloads one photo, addressed by `node_uid` (see [`photo_timeline`]) via
/// the synthetic `/photos/<uid>` path `photo download` accepts — confirmed
/// live that a bare uid or a `filesystem`-family call with the same uid are
/// both rejected ("not supported"), only this exact prefixed form works.
/// Forces `remove` for the same "fresh temp dir, no interactive prompts"
/// reasoning as [`download`]. The CLI names the downloaded file by its own
/// decrypted name, which the caller must read back rather than assume —
/// see `crate::photos`'s disambiguation of same-named photos.
pub fn photo_download(
    runner: &dyn CommandRunner,
    node_uid: &str,
    local_folder: &Path,
) -> Result<TransferSummary, DriveError> {
    let remote_path = format!("/photos/{node_uid}");
    let local = local_folder.to_string_lossy();
    let out = runner.run(
        &[
            "photo",
            "download",
            "-j",
            "-c",
            "remove",
            &remote_path,
            &local,
        ],
        TRANSFER_TIMEOUT,
    )?;
    ensure_success(&remote_path, &out)?;
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

/// Argument-building half of [`upload`] — see [`download_args`]'s doc
/// comment for why this is split out.
pub(crate) fn upload_args(local_path: &Path, parent_path: &str) -> Vec<String> {
    vec![
        "filesystem".to_string(),
        "upload".to_string(),
        "-j".to_string(),
        "-f".to_string(),
        "replace".to_string(),
        escape_glob_metacharacters(&local_path.to_string_lossy()),
        parent_path.to_string(),
    ]
}

/// Result-checking half of [`upload`] — see [`finish_download`]'s doc
/// comment for why this is split out.
pub(crate) fn finish_upload(
    parent_path: &str,
    out: CommandOutput,
) -> Result<TransferSummary, DriveError> {
    ensure_success(parent_path, &out)?;
    let summary: TransferSummary = serde_json::from_str(&out.stdout)?;
    ensure_no_failures(&format!("upload to {parent_path}"), &summary)?;
    Ok(summary)
}

/// Uploads `local_path` into `parent_path`. Forces `replace` for the same
/// reason as [`download`] — an interactive prompt would hang the worker.
pub fn upload(
    runner: &dyn CommandRunner,
    local_path: &Path,
    parent_path: &str,
) -> Result<TransferSummary, DriveError> {
    let args = upload_args(local_path, parent_path);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = runner.run(&arg_refs, TRANSFER_TIMEOUT)?;
    finish_upload(parent_path, out)
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

/// Restores a trashed item to wherever it was before being trashed — the CLI
/// takes no destination argument, it remembers the original location itself.
/// Same `[{uid, ok}]` response shape as [`trash_path`] (confirmed live).
pub fn restore_path(runner: &dyn CommandRunner, path: &str) -> Result<(), DriveError> {
    let out = runner.run(&["filesystem", "restore", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    let outcomes: Vec<TrashOutcome> = serde_json::from_str(&out.stdout)?;
    if let Some(failed) = outcomes.iter().find(|o| !o.ok) {
        return Err(DriveError::Cli(format!("failed to restore {}", failed.uid)));
    }
    Ok(())
}

/// Permanently deletes an already-trashed item (the CLI refuses this on
/// anything not already in `/trash` — matches [`trash_path`] first, this is
/// only ever called on a path already known to be under `/trash`, see
/// `worker/protondriveworker.cpp`'s `del()`). Same response shape as
/// [`trash_path`] (confirmed live).
pub fn permanently_delete_path(runner: &dyn CommandRunner, path: &str) -> Result<(), DriveError> {
    let out = runner.run(&["filesystem", "delete", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    let outcomes: Vec<TrashOutcome> = serde_json::from_str(&out.stdout)?;
    if let Some(failed) = outcomes.iter().find(|o| !o.ok) {
        return Err(DriveError::Cli(format!(
            "failed to permanently delete {}",
            failed.uid
        )));
    }
    Ok(())
}

/// Permanently deletes everything in `/trash` (not `/photos-trash`, per the
/// CLI's own `--help`). Asynchronous on Proton's side — a successful return
/// here doesn't mean `/trash` is already empty, just that the request was
/// accepted.
pub fn empty_trash(runner: &dyn CommandRunner) -> Result<(), DriveError> {
    let out = runner.run(&["filesystem", "empty-trash", "-j"], METADATA_TIMEOUT)?;
    ensure_success("/trash", &out)
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

/// Splits a Drive path into (parent, name), e.g. "/my-files/sub/a.txt" ->
/// ("/my-files/sub", "a.txt"), and "/my-files" -> ("/", "my-files").
fn split_parent_and_name(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some(("", name)) => ("/".to_string(), name.to_string()),
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Handles KIO's `rename(src, dest)`, called for both an in-place rename
/// (Dolphin's F2) and a same-protocol move (drag-and-drop between two
/// `protondrive:/` folders) — KIO doesn't distinguish the two, both arrive
/// here as a single (src, dest) pair. The `proton-drive` CLI, unlike KIO,
/// treats them as genuinely separate operations with no combined "move and
/// rename" call (`filesystem rename` changes the name in place;
/// `filesystem move` changes the parent, keeping the name) — so when both
/// the parent and the name change, this makes two sequential CLI calls:
/// move first (to the destination parent, still under the old name), then
/// rename at the new location.
pub fn rename_or_move(
    runner: &dyn CommandRunner,
    old_path: &str,
    new_path: &str,
) -> Result<(), DriveError> {
    let (old_parent, old_name) = split_parent_and_name(old_path);
    let (new_parent, new_name) = split_parent_and_name(new_path);

    if old_parent == new_parent {
        rename_path(runner, old_path, &new_name)?;
        return Ok(());
    }

    move_path(runner, old_path, &new_parent)?;
    if old_name == new_name {
        return Ok(());
    }

    let moved_path = format!("{}/{}", new_parent.trim_end_matches('/'), old_name);
    rename_path(runner, &moved_path, &new_name)?;
    Ok(())
}

/// Members, pending invitations, and public link settings for a node — see
/// `crate::sharing` for the merged, worker-friendly view built on top of
/// this raw response.
pub fn sharing_status(runner: &dyn CommandRunner, path: &str) -> Result<SharingStatus, DriveError> {
    let out = runner.run(&["sharing", "status", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    // A node that has never been shared prints the literal text "undefined"
    // instead of an empty status object — confirmed live (a
    // `JSON.stringify(undefined)` result from the CLI's own JS
    // implementation, not something exit-code/stderr flags as an error).
    // Every never-shared file hits this on the first "Share" click, so
    // treating it as anything but the obvious "nothing shared yet" would
    // break the dialog for the common case rather than the edge case.
    if out.stdout.trim() == "undefined" {
        return Ok(SharingStatus::default());
    }
    Ok(serde_json::from_str(&out.stdout)?)
}

/// Invites a single user by email, or updates their role if the node is
/// already shared with them (matches the CLI's own `sharing invite --help`
/// wording). `message`, when non-empty, is included as clear text in the
/// invitation email (`-m`).
pub fn sharing_invite(
    runner: &dyn CommandRunner,
    path: &str,
    email: &str,
    role: &str,
    message: &str,
) -> Result<(), DriveError> {
    let mut args = vec!["sharing", "invite", "-j", "-u", email, "-r", role];
    if !message.is_empty() {
        args.push("-m");
        args.push(message);
    }
    args.push(path);
    let out = runner.run(&args, METADATA_TIMEOUT)?;
    ensure_success(path, &out)
}

/// Removes one user's access (or pending invitation) by email.
pub fn sharing_remove_member(
    runner: &dyn CommandRunner,
    path: &str,
    email: &str,
) -> Result<(), DriveError> {
    let out = runner.run(
        &["sharing", "remove", "-j", "-e", email, path],
        METADATA_TIMEOUT,
    )?;
    ensure_success(path, &out)
}

/// Creates or updates the node's public link. `password`/`expiration` are
/// forwarded only when non-empty — the CLI treats their absence as "no
/// password"/"no expiration".
pub fn sharing_set_link(
    runner: &dyn CommandRunner,
    path: &str,
    role: &str,
    password: &str,
    expiration: &str,
) -> Result<PublicLink, DriveError> {
    let mut args = vec!["sharing", "set-url", "-j", "--role", role];
    if !password.is_empty() {
        args.push("--password");
        args.push(password);
    }
    if !expiration.is_empty() {
        args.push("--expiration");
        args.push(expiration);
    }
    args.push(path);
    let out = runner.run(&args, METADATA_TIMEOUT)?;
    ensure_success(path, &out)?;
    // set-url returns the whole sharing-status object, not a flat link —
    // confirmed live (see `SharingStatus::url_access`'s doc comment).
    let status: SharingStatus = serde_json::from_str(&out.stdout)?;
    status.url_access.ok_or_else(|| {
        DriveError::Parse(format!(
            "sharing set-url for {path} succeeded but returned no urlAccess"
        ))
    })
}

/// Removes the node's public link; direct member access is unaffected (per
/// the CLI's own `sharing remove-url --help`).
pub fn sharing_remove_link(runner: &dyn CommandRunner, path: &str) -> Result<(), DriveError> {
    let out = runner.run(&["sharing", "remove-url", "-j", path], METADATA_TIMEOUT)?;
    ensure_success(path, &out)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn is_transient_lock_contention_matches_the_cli_own_sqlite_cache_lock_error() {
        let locked = CommandOutput {
            stdout: "SQLiteError: database is locked".to_string(),
            stderr: String::new(),
            success: false,
        };
        assert!(is_transient_lock_contention(&locked));

        let busy = CommandOutput {
            stdout: String::new(),
            stderr: "code: 'SQLITE_BUSY_RECOVERY'".to_string(),
            success: false,
        };
        assert!(is_transient_lock_contention(&busy));
    }

    #[test]
    fn is_transient_lock_contention_ignores_unrelated_failures_and_successes() {
        let unrelated = CommandOutput {
            stdout: String::new(),
            stderr: "internal server error, please retry".to_string(),
            success: false,
        };
        assert!(!is_transient_lock_contention(&unrelated));

        let succeeded_but_mentions_it = CommandOutput {
            stdout: "no database is locked here".to_string(),
            stderr: String::new(),
            success: true,
        };
        assert!(!is_transient_lock_contention(&succeeded_but_mentions_it));
    }

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
    fn restore_path_succeeds_when_all_outcomes_are_ok() {
        let runner = MockRunner::success(TRASH_OK);
        restore_path(&runner, "/trash/Photos").unwrap();
    }

    #[test]
    fn restore_path_errors_when_any_outcome_failed() {
        let runner = MockRunner::success(TRASH_PARTIAL_FAILURE);
        let err = restore_path(&runner, "/trash").unwrap_err();
        assert!(matches!(err, DriveError::Cli(_)));
    }

    #[test]
    fn permanently_delete_path_succeeds_when_all_outcomes_are_ok() {
        let runner = MockRunner::success(TRASH_OK);
        permanently_delete_path(&runner, "/trash/Photos").unwrap();
    }

    #[test]
    fn permanently_delete_path_errors_when_any_outcome_failed() {
        let runner = MockRunner::success(TRASH_PARTIAL_FAILURE);
        let err = permanently_delete_path(&runner, "/trash").unwrap_err();
        assert!(matches!(err, DriveError::Cli(_)));
    }

    #[test]
    fn empty_trash_succeeds_on_a_successful_cli_call() {
        let runner = MockRunner::success("");
        empty_trash(&runner).unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec!["filesystem", "empty-trash", "-j"]
        );
    }

    #[test]
    fn empty_trash_propagates_a_cli_failure() {
        let runner = MockRunner::failure("internal server error");
        let err = empty_trash(&runner).unwrap_err();
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
    fn rename_or_move_renames_in_place_when_only_the_name_changes() {
        const RENAMED: &str = r#"{
            "uid":"uid-file",
            "name":{"ok":true,"value":"new-name.txt"},
            "type":"file",
            "isShared":false,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z"
        }"#;
        let runner = MockRunner::success(RENAMED);
        rename_or_move(&runner, "/my-files/old-name.txt", "/my-files/new-name.txt").unwrap();
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
    fn rename_or_move_moves_when_only_the_parent_changes() {
        let runner = MockRunner::success(TRASH_OK);
        rename_or_move(&runner, "/my-files/a.txt", "/my-files/Sub/a.txt").unwrap();
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
    fn rename_or_move_moves_then_renames_when_both_change() {
        const RENAMED: &str = r#"{
            "uid":"uid-file",
            "name":{"ok":true,"value":"b.txt"},
            "type":"file",
            "isShared":false,
            "creationTime":"2026-01-01T00:00:00.000Z",
            "modificationTime":"2026-01-01T00:00:00.000Z"
        }"#;
        let runner = ScriptedRunner::new(vec![success(TRASH_OK), success(RENAMED)]);
        rename_or_move(&runner, "/my-files/a.txt", "/my-files/Sub/b.txt").unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            vec![
                vec![
                    "filesystem",
                    "move",
                    "-j",
                    "/my-files/a.txt",
                    "/my-files/Sub"
                ],
                vec!["filesystem", "rename", "-j", "/my-files/Sub/a.txt", "b.txt"],
            ]
        );
    }

    #[test]
    fn rename_or_move_does_not_rename_when_the_move_fails() {
        let runner = MockRunner::success(TRASH_PARTIAL_FAILURE);
        let err = rename_or_move(&runner, "/my-files/a.txt", "/my-files/Sub/b.txt").unwrap_err();
        assert!(matches!(err, DriveError::Cli(_)));
    }

    #[test]
    fn rename_or_move_treats_a_root_level_destination_parent_as_slash() {
        let runner = MockRunner::success(TRASH_OK);
        rename_or_move(&runner, "/my-files/Sub/a.txt", "/a.txt").unwrap();
        assert_eq!(
            *runner.last_args.borrow(),
            vec!["filesystem", "move", "-j", "/my-files/Sub/a.txt", "/",]
        );
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

    #[test]
    fn sharing_set_link_extracts_url_access_from_the_whole_status_response() {
        // Real `sharing set-url -j` output, captured live: the CLI returns
        // the whole sharing-status object (same shape as `sharing status`),
        // not a flat link — the link itself is nested under `urlAccess`.
        const SET_URL_OUTPUT: &str = r#"{
            "protonInvitations":[],
            "nonProtonInvitations":[],
            "members":[],
            "urlAccess":{
                "uid":"uid-url-access",
                "creationTime":"2026-08-20T09:41:32.000Z",
                "expirationTime":"2026-09-01T00:00:00.000Z",
                "role":"viewer",
                "url":"https://drive.proton.me/urls/Y14XRXP714#MWiP4V07VZtv",
                "creatorEmail":"famille@biton-collomb.fr",
                "numberOfInitializedDownloads":3
            },
            "editorsCanShare":false
        }"#;
        let runner = MockRunner::success(SET_URL_OUTPUT);
        let link =
            sharing_set_link(&runner, "/my-files/report.pdf", "viewer", "", "2026-09-01").unwrap();
        assert_eq!(
            link.url,
            "https://drive.proton.me/urls/Y14XRXP714#MWiP4V07VZtv"
        );
        assert_eq!(link.role, "viewer");
        assert_eq!(
            link.expiration_time.as_deref(),
            Some("2026-09-01T00:00:00.000Z")
        );
        assert_eq!(link.number_of_initialized_downloads, 3);
    }

    #[test]
    fn sharing_set_link_errors_if_the_response_has_no_url_access() {
        let runner = MockRunner::success(
            r#"{"protonInvitations":[],"nonProtonInvitations":[],"members":[],"editorsCanShare":false}"#,
        );
        assert!(sharing_set_link(&runner, "/my-files/report.pdf", "viewer", "", "").is_err());
    }

    #[test]
    fn sharing_status_treats_the_cli_literal_undefined_output_as_an_empty_status() {
        // Confirmed live: `sharing status -j` on any node that has never
        // been shared prints the literal text "undefined" (not JSON, exit
        // 0) — reproduced on both plain /my-files files and /photos items,
        // so it's the CLI's own "nothing shared yet" quirk, not a path- or
        // node-type-specific failure.
        let runner = MockRunner::success("undefined");
        let status = sharing_status(&runner, "/my-files/never-shared.pdf").unwrap();
        assert!(status.members.is_empty());
        assert!(status.proton_invitations.is_empty());
        assert!(status.non_proton_invitations.is_empty());
        assert!(status.url_access.is_none());
        assert!(!status.editors_can_share);
    }
}
