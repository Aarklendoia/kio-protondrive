//! Persistent index of explicitly **pinned** files — a pinned remote path
//! always has a fresh-ish local copy under [`Cache::default_root`], so the
//! KIO worker can serve `get`/`stat` for it instantly instead of shelling
//! out to the CLI every time (see issue #30). Everything else under
//! `protondrive:/` stays exactly as on-demand/ephemeral as before — nothing
//! is cached opportunistically, only what's explicitly pinned, which is
//! also why there's no auto-eviction policy here: "cleanup" is just
//! unpinning.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::cli::{self, CommandRunner, DriveError};

/// How long a writer waits for `SQLITE_BUSY` to clear before giving up.
/// Matters here specifically because, unlike a single-connection design,
/// `daemon/src/control.rs`'s pin/unpin HTTP routes open their own
/// short-lived `Cache::open()` (a separate connection to the same file)
/// per request, independent of the long-lived one `main.rs` holds for the
/// sync loop — without this, either writer hitting the other mid-write
/// gets an immediate error instead of a chance to wait its turn.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRecord {
    pub remote_path: String,
    pub local_path: PathBuf,
    pub local_mtime: i64,
    pub local_size: i64,
    pub last_synced_at: i64,
}

pub struct Cache {
    conn: Connection,
    root: PathBuf,
}

impl Cache {
    pub fn open(db_path: &Path, root: &Path) -> Result<Self, DriveError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DriveError::Io(e.to_string()))?;
        }
        std::fs::create_dir_all(root).map_err(|e| DriveError::Io(e.to_string()))?;
        let conn = Connection::open(db_path).map_err(|e| DriveError::Sqlite(e.to_string()))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pins (
                remote_path TEXT PRIMARY KEY,
                local_path TEXT NOT NULL,
                local_mtime INTEGER NOT NULL,
                local_size INTEGER NOT NULL,
                last_synced_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        // lookup_by_local_path (every upload_if_needed/reconcile call) would
        // otherwise full-scan this table — cheap now, but O(n) per call
        // times O(n) pinned files at startup reconcile is O(n^2).
        conn.execute(
            "CREATE INDEX IF NOT EXISTS pins_local_path_idx ON pins(local_path)",
            [],
        )
        .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(Self {
            conn,
            root: root.to_path_buf(),
        })
    }

    /// `$XDG_DATA_HOME/kio-protondrive/cache-index.sqlite3` (falls back to
    /// `~/.local/share`) — persistent: pin *state* is user intent, unlike
    /// the cached bytes themselves (see [`Self::default_root`]).
    pub fn default_db_path() -> PathBuf {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".local").join("share")
            });
        data_home
            .join("kio-protondrive")
            .join("cache-index.sqlite3")
    }

    /// `$XDG_CACHE_HOME/kio-protondrive/files/` (falls back to `~/.cache`)
    /// — regenerable: safe to delete wholesale, a pinned file just
    /// re-downloads on next access if its local copy is gone ([`Self::lookup`]
    /// checks the file still exists, not just the DB row).
    pub fn default_root() -> PathBuf {
        let cache_home = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".cache")
            });
        cache_home.join("kio-protondrive").join("files")
    }

    /// The cache root this instance was opened with — lets callers (the
    /// daemon) derive a local cache path's would-be remote path by
    /// stripping this prefix, without needing to track the root separately.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Local cache path for `remote_path`, if it's pinned *and* the file
    /// still actually exists on disk (a wiped `~/.cache` shouldn't report
    /// stale hits).
    pub fn lookup(&self, remote_path: &str) -> Result<Option<PathBuf>, DriveError> {
        let local: Option<String> = self
            .conn
            .query_row(
                "SELECT local_path FROM pins WHERE remote_path = ?1",
                params![remote_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(local.map(PathBuf::from).filter(|p| p.is_file()))
    }

    /// Reverse lookup: given a local cache path (as seen by the daemon's
    /// filesystem watcher), which remote path does it belong to?
    pub fn lookup_by_local_path(&self, local_path: &Path) -> Result<Option<String>, DriveError> {
        let key = local_path.to_string_lossy();
        self.conn
            .query_row(
                "SELECT remote_path FROM pins WHERE local_path = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))
    }

    /// Downloads `remote_path` into the cache root (mirroring its
    /// directory structure) and records it as pinned. Idempotent: pinning
    /// an already-pinned path just re-downloads and updates the record —
    /// unless the existing local copy has unsynced edits (`needs_upload`),
    /// in which case this refuses rather than silently overwriting them
    /// with the (older) remote content; pass `force` to discard them
    /// anyway. Only single files can be pinned: recursively downloading and
    /// tracking an entire folder as one row isn't supported (see
    /// [`Self::lookup`]'s file-only filter), so a folder is rejected before
    /// any download happens.
    pub fn pin(
        &self,
        runner: &dyn CommandRunner,
        remote_path: &str,
        force: bool,
    ) -> Result<PathBuf, DriveError> {
        let entry = cli::stat_path(runner, remote_path)?;
        if entry.is_folder() {
            return Err(DriveError::Cli(format!(
                "{remote_path} is a folder — only individual files can be pinned"
            )));
        }

        if !force {
            if let Some(existing_local) = self.lookup(remote_path)? {
                if self.is_dirty(&existing_local, remote_path)? {
                    return Err(DriveError::Cli(format!(
                        "{remote_path} has unsynced local changes — upload them first, or pin \
                         again with force to discard them"
                    )));
                }
            }
        }

        let rel = remote_path.trim_start_matches('/');
        let target_dir = match Path::new(rel).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => self.root.join(parent),
            _ => self.root.clone(),
        };
        std::fs::create_dir_all(&target_dir).map_err(|e| DriveError::Io(e.to_string()))?;
        cli::download(runner, remote_path, &target_dir)?;

        let file_name = Path::new(rel).file_name().ok_or_else(|| {
            DriveError::Cli(format!("cannot pin a bare root path: {remote_path}"))
        })?;
        let local_path = target_dir.join(file_name);
        let metadata = std::fs::metadata(&local_path).map_err(|e| DriveError::Io(e.to_string()))?;
        let (mtime, size) = metadata_mtime_size(&metadata)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn
            .execute(
                "INSERT INTO pins (remote_path, local_path, local_mtime, local_size, last_synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(remote_path) DO UPDATE SET
                    local_path = excluded.local_path,
                    local_mtime = excluded.local_mtime,
                    local_size = excluded.local_size,
                    last_synced_at = excluded.last_synced_at",
                params![remote_path, local_path.to_string_lossy(), mtime, size, now],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;

        Ok(local_path)
    }

    /// Un-pins `remote_path`: deletes the local cached copy (the remote
    /// file on Drive is untouched) and removes the record. A no-op if it
    /// wasn't pinned. Refuses (leaving both the file and the record intact)
    /// if the local copy has unsynced edits, unless `force` is set — same
    /// discard-guard as [`Self::pin`]. If the file itself can't actually be
    /// removed (permissions, read-only mount — anything other than it
    /// already being gone), the record is kept too rather than silently
    /// losing track of an orphaned local file.
    pub fn unpin(&self, remote_path: &str, force: bool) -> Result<(), DriveError> {
        if let Some(local_path) = self.lookup(remote_path)? {
            if !force && self.is_dirty(&local_path, remote_path)? {
                return Err(DriveError::Cli(format!(
                    "{remote_path} has unsynced local changes — upload them first, or unpin \
                     again with force to discard them"
                )));
            }
            match std::fs::remove_file(&local_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(DriveError::Io(err.to_string())),
            }
        }
        self.conn
            .execute(
                "DELETE FROM pins WHERE remote_path = ?1",
                params![remote_path],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Whether `local_path`'s current on-disk state differs from what was
    /// last recorded as synced for `remote_path` — same check
    /// [`Self::needs_upload`] does, but stats the file itself instead of
    /// taking mtime/size from the caller (used by [`Self::pin`]/
    /// [`Self::unpin`], which have no watcher event to read them from).
    fn is_dirty(&self, local_path: &Path, remote_path: &str) -> Result<bool, DriveError> {
        let metadata = match std::fs::metadata(local_path) {
            Ok(metadata) => metadata,
            // Already gone locally — nothing to lose, safe to proceed.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(DriveError::Io(err.to_string())),
        };
        let (mtime, size) = metadata_mtime_size(&metadata)?;
        self.needs_upload(remote_path, mtime, size)
    }

    /// Rekeys a pinned entry when its cache file is renamed/moved locally.
    /// `new_remote_path` is normally the path it was *actually* renamed to
    /// on Drive too; a caller that couldn't propagate the rename remotely
    /// (see the daemon's `handle_rename` fallback) can pass the unchanged
    /// `old_remote_path` back here instead, to at least keep tracking the
    /// entry under its new local location without pretending Drive was
    /// updated. Content (mtime/size) carries over unchanged — a rename
    /// doesn't change file content.
    pub fn rename(
        &self,
        old_remote_path: &str,
        new_remote_path: &str,
        new_local_path: &Path,
    ) -> Result<(), DriveError> {
        self.conn
            .execute(
                "UPDATE pins SET remote_path = ?1, local_path = ?2 WHERE remote_path = ?3",
                params![
                    new_remote_path,
                    new_local_path.to_string_lossy(),
                    old_remote_path
                ],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Records `remote_path` as freshly uploaded — call after a successful
    /// re-upload of a pinned file's local edits, mirroring
    /// [`Self::needs_upload`]'s mtime/size tracking.
    pub fn mark_synced(
        &self,
        remote_path: &str,
        local_mtime: i64,
        local_size: i64,
    ) -> Result<(), DriveError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn
            .execute(
                "UPDATE pins SET local_mtime = ?2, local_size = ?3, last_synced_at = ?4 \
                 WHERE remote_path = ?1",
                params![remote_path, local_mtime, local_size, now],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Whether a pinned file's local copy has changed since it was last
    /// recorded as synced — `false` if `remote_path` isn't actually pinned
    /// (nothing to upload).
    pub fn needs_upload(
        &self,
        remote_path: &str,
        local_mtime: i64,
        local_size: i64,
    ) -> Result<bool, DriveError> {
        let record: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT local_mtime, local_size FROM pins WHERE remote_path = ?1",
                params![remote_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(match record {
            Some((m, s)) => m != local_mtime || s != local_size,
            None => false,
        })
    }

    /// Every currently pinned path — the daemon's reconcile pass walks
    /// this to catch local edits made while it wasn't running.
    pub fn all_pinned(&self) -> Result<Vec<PinRecord>, DriveError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT remote_path, local_path, local_mtime, local_size, last_synced_at FROM pins",
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PinRecord {
                    remote_path: row.get(0)?,
                    local_path: PathBuf::from(row.get::<_, String>(1)?),
                    local_mtime: row.get(2)?,
                    local_size: row.get(3)?,
                    last_synced_at: row.get(4)?,
                })
            })
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| DriveError::Sqlite(e.to_string()))
    }
}

/// Extracts (mtime as unix seconds, size in bytes) — the pair every pin
/// record tracks — from a file's metadata.
fn metadata_mtime_size(metadata: &std::fs::Metadata) -> Result<(i64, i64), DriveError> {
    let mtime = metadata
        .modified()
        .map_err(|e| DriveError::Io(e.to_string()))?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    Ok((mtime, metadata.len() as i64))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::cli::CommandOutput;

    /// Simulates both `filesystem info` (the folder-vs-file pre-check
    /// `Cache::pin` now does before downloading) and `filesystem download`'s
    /// real side effect (a file actually appearing under the target folder,
    /// since `Cache::pin` reads the downloaded file's real metadata
    /// afterward) — a mock that only returns canned JSON without touching
    /// the filesystem isn't enough here, unlike the plainer mocks in
    /// `cli.rs`'s own tests.
    struct DownloadingMockRunner {
        content: &'static [u8],
        is_folder: bool,
    }

    impl DownloadingMockRunner {
        fn file(content: &'static [u8]) -> Self {
            Self {
                content,
                is_folder: false,
            }
        }

        fn folder() -> Self {
            Self {
                content: b"",
                is_folder: true,
            }
        }
    }

    impl CommandRunner for DownloadingMockRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
            if args[0..2] == ["filesystem", "info"] {
                let node_type = if self.is_folder { "folder" } else { "file" };
                return Ok(CommandOutput {
                    stdout: format!(
                        r#"{{"uid":"uid-1","name":{{"ok":true,"value":"x"}},"type":"{node_type}","isShared":false,"creationTime":"2026-01-01T00:00:00.000Z","modificationTime":"2026-01-01T00:00:00.000Z"}}"#
                    ),
                    stderr: String::new(),
                    success: true,
                });
            }
            if args[0..2] == ["filesystem", "download"] {
                let remote_path = args[args.len() - 2];
                let local_folder = args[args.len() - 1];
                let file_name = Path::new(remote_path).file_name().unwrap();
                std::fs::write(Path::new(local_folder).join(file_name), self.content).unwrap();
                return Ok(CommandOutput {
                    stdout: r#"{"transferredItems":1,"transferredBytes":1,"skippedItems":0,"failedItems":0,"failures":[]}"#.to_string(),
                    stderr: String::new(),
                    success: true,
                });
            }
            unreachable!("unexpected command: {args:?}")
        }
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            Cache::open(&dir.path().join("index.sqlite3"), &dir.path().join("files")).unwrap();
        (dir, cache)
    }

    #[test]
    fn pin_downloads_and_records_the_file() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");

        let local_path = cache
            .pin(&runner, "/my-files/Reports/q3.pdf", false)
            .unwrap();

        assert!(local_path.ends_with("Reports/q3.pdf"));
        assert_eq!(std::fs::read(&local_path).unwrap(), b"hello");
        assert_eq!(
            cache.lookup("/my-files/Reports/q3.pdf").unwrap(),
            Some(local_path)
        );
    }

    #[test]
    fn pin_rejects_a_folder_without_downloading_anything() {
        let (dir, cache) = cache();
        let runner = DownloadingMockRunner::folder();

        let err = cache.pin(&runner, "/my-files/Reports", false).unwrap_err();

        assert!(err.to_string().contains("folder"));
        assert_eq!(cache.lookup("/my-files/Reports").unwrap(), None);
        // Nothing was written under the cache root at all — the rejection
        // happens before create_dir_all/download, not as cleanup after.
        assert!(!dir.path().join("files/my-files/Reports").exists());
    }

    #[test]
    fn pin_refuses_to_overwrite_unsynced_local_edits() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        std::fs::write(&local_path, b"locally edited, not yet uploaded").unwrap();

        let err = cache.pin(&runner, "/my-files/a.txt", false).unwrap_err();

        assert!(err.to_string().contains("unsynced"));
        // The local edit survived — pin() bailed before re-downloading.
        assert_eq!(
            std::fs::read(&local_path).unwrap(),
            b"locally edited, not yet uploaded"
        );
    }

    #[test]
    fn pin_with_force_overwrites_unsynced_local_edits() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        std::fs::write(&local_path, b"locally edited, not yet uploaded").unwrap();

        cache.pin(&runner, "/my-files/a.txt", true).unwrap();

        assert_eq!(std::fs::read(&local_path).unwrap(), b"hello");
    }

    #[test]
    fn lookup_returns_none_for_something_never_pinned() {
        let (_dir, cache) = cache();
        assert_eq!(cache.lookup("/my-files/never-pinned.txt").unwrap(), None);
    }

    #[test]
    fn lookup_returns_none_if_the_local_file_was_deleted_out_from_under_it() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();

        std::fs::remove_file(&local_path).unwrap();

        assert_eq!(cache.lookup("/my-files/a.txt").unwrap(), None);
    }

    #[test]
    fn lookup_by_local_path_finds_the_remote_path() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();

        assert_eq!(
            cache.lookup_by_local_path(&local_path).unwrap(),
            Some("/my-files/a.txt".to_string())
        );
    }

    #[test]
    fn unpin_deletes_the_local_file_and_the_record() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();

        cache.unpin("/my-files/a.txt", false).unwrap();

        assert!(!local_path.exists());
        assert_eq!(cache.lookup("/my-files/a.txt").unwrap(), None);
    }

    #[test]
    fn unpin_is_a_noop_for_something_never_pinned() {
        let (_dir, cache) = cache();
        cache.unpin("/my-files/never-pinned.txt", false).unwrap();
    }

    #[test]
    fn unpin_refuses_to_discard_unsynced_local_edits() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        std::fs::write(&local_path, b"locally edited, not yet uploaded").unwrap();

        let err = cache.unpin("/my-files/a.txt", false).unwrap_err();

        assert!(err.to_string().contains("unsynced"));
        // Neither the file nor the record was touched.
        assert!(local_path.exists());
        assert_eq!(cache.lookup("/my-files/a.txt").unwrap(), Some(local_path));
    }

    #[test]
    fn unpin_with_force_discards_unsynced_local_edits() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        let local_path = cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        std::fs::write(&local_path, b"locally edited, not yet uploaded").unwrap();

        cache.unpin("/my-files/a.txt", true).unwrap();

        assert!(!local_path.exists());
        assert_eq!(cache.lookup("/my-files/a.txt").unwrap(), None);
    }

    #[test]
    fn rename_rekeys_both_remote_and_local_path() {
        let (dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/old.txt", false).unwrap();
        let new_local = dir.path().join("files/my-files/new.txt");

        cache
            .rename("/my-files/old.txt", "/my-files/new.txt", &new_local)
            .unwrap();

        assert_eq!(cache.lookup("/my-files/old.txt").unwrap(), None);
        assert_eq!(
            cache.lookup_by_local_path(&new_local).unwrap(),
            Some("/my-files/new.txt".to_string())
        );
    }

    #[test]
    fn rename_can_keep_the_same_remote_path_when_only_the_local_side_moved() {
        let (dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        let new_local = dir.path().join("files/my-files/a-moved.txt");

        cache
            .rename("/my-files/a.txt", "/my-files/a.txt", &new_local)
            .unwrap();

        assert_eq!(
            cache.lookup_by_local_path(&new_local).unwrap(),
            Some("/my-files/a.txt".to_string())
        );
    }

    #[test]
    fn needs_upload_is_false_for_something_not_pinned() {
        let (_dir, cache) = cache();
        assert!(!cache.needs_upload("/my-files/a.txt", 100, 200).unwrap());
    }

    #[test]
    fn needs_upload_reflects_mtime_size_changes_since_pin() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        let record = cache.all_pinned().unwrap().into_iter().next().unwrap();

        assert!(!cache
            .needs_upload("/my-files/a.txt", record.local_mtime, record.local_size)
            .unwrap());
        assert!(cache
            .needs_upload("/my-files/a.txt", record.local_mtime + 1, record.local_size)
            .unwrap());
    }

    #[test]
    fn mark_synced_updates_the_record_needs_upload_checks_against() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/a.txt", false).unwrap();

        cache.mark_synced("/my-files/a.txt", 555, 777).unwrap();

        assert!(!cache.needs_upload("/my-files/a.txt", 555, 777).unwrap());
        assert!(cache.needs_upload("/my-files/a.txt", 556, 777).unwrap());
    }

    #[test]
    fn all_pinned_lists_every_pin() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/a.txt", false).unwrap();
        cache.pin(&runner, "/my-files/sub/b.txt", false).unwrap();

        let mut paths: Vec<String> = cache
            .all_pinned()
            .unwrap()
            .into_iter()
            .map(|r| r.remote_path)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["/my-files/a.txt", "/my-files/sub/b.txt"]);
    }
}
