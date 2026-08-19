//! Persistent index of explicitly **pinned** files — a pinned remote path
//! always has a fresh-ish local copy under [`Cache::default_root`], so the
//! KIO worker can serve `get`/`stat` for it instantly instead of shelling
//! out to the CLI every time (see issue #30). Pinned files are never
//! auto-evicted: "cleanup" there is just unpinning.
//!
//! Also holds a second, unrelated cache: [`Self::fresh_photo_timeline`] /
//! [`Self::store_photo_timeline`] memoize `/photos`'s full node listing (see
//! `crate::photos`'s doc comment for why that's expensive) in the same
//! on-disk SQLite file rather than an in-process cache — this file is
//! already shared/lock-safe across every process that opens it (multiple
//! KIO worker instances, the daemon), which per-process memory isn't:
//! confirmed live that Dolphin's `kio-fuse` mount spawns short-lived,
//! disposable worker processes for `/photos` thumbnailing, each of which
//! would otherwise pay the ~80s cold-start cost independently and get
//! killed by an impatient caller before finishing, in a loop.
//!
//! And a third: [`Self::cached_stat`]/[`Self::store_stat`]/[`Self::cached_listing`]/
//! [`Self::store_listing`] cache general `stat`/`list_dir` results for real
//! Drive paths (see issue #8) — unlike the photo timeline above, these have
//! **no TTL**: a hit is always served, however old. `crate::bridge`'s
//! read-through/write-through logic populates this on a genuine cache miss;
//! staying fresh after that is the sync daemon's job (a periodic sweep,
//! `daemon/src/fs_refresh.rs`) plus this app's own writes invalidating what
//! they touch (`crate::bridge`'s `make_dir`/`upload_from`/`trash`/
//! `rename_or_move`) — never a background thread spawned from the worker
//! itself, since a KIO worker process is short-lived and poolable and can be
//! killed mid-thread. This is a real, deliberate deviation from
//! `docs/DESIGN.md`'s on-demand/stateless philosophy — see that doc's cache
//! section for the consistency tradeoff this accepts (an external rename or
//! delete can lag behind by up to the daemon's sweep interval).
//!
//! And a fourth: [`Self::cached_file`]/[`Self::store_cached_file`]/
//! [`Self::touch_cached_file`]/[`Self::evict_cached_file`] are an
//! **opportunistic** cache of downloaded/uploaded file bytes (see issue
//! #60) — every `get()`/`put()`, not just explicitly pinned paths, get a
//! row here. Deliberately a separate table from `pins` rather than a
//! `pinned` flag on the same one: `pins`' existing semantics (`unpin()`
//! deletes immediately, `pin()` never expires on its own) stay completely
//! unchanged, and a pinned path simply never gets a `cached_files` row in
//! the first place (`crate::bridge`'s `get()`/`put()` check the pin table
//! first). Unlike `pins`, a hit here is re-verified against the remote's
//! `modification_time` before being trusted (`crate::bridge`'s
//! `lookup_cached`) — a file can change from another device/the web app
//! without the user pinning anything to be told about it, so blindly
//! trusting a stale local copy here would be a correctness bug, not just a
//! staleness tradeoff. Eviction is age-based on `last_accessed_at` (not
//! when it was first cached), swept periodically by the daemon
//! (`daemon/src/cache_eviction.rs`) against a user-configurable retention
//! window (`daemon/src/config.rs`'s `cache_retention_days`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::cli::{self, CommandRunner, DriveError};
use crate::entry::NodeEntry;

/// How stale a cached `/photos` timeline can be before a fresh
/// `photo timeline -d` CLI call replaces it.
const PHOTO_TIMELINE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

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
        // Single-row table (`id = 1` always): there's only ever one
        // account's worth of /photos to cache per install, so a full
        // remote_path-keyed table like `pins` would be needless.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS photo_timeline_cache (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                fetched_at INTEGER NOT NULL,
                payload TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_stat_cache (
                path TEXT PRIMARY KEY,
                node_json TEXT NOT NULL,
                fetched_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_listing_cache (
                parent_path TEXT PRIMARY KEY,
                children_json TEXT NOT NULL,
                fetched_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        // Opportunistic cache of downloaded/uploaded file *bytes* (#60) —
        // separate from `pins` (explicit, permanent, never auto-evicted):
        // a row here is created on any get()/put() and swept away by the
        // daemon once its last_accessed_at ages past the configured
        // retention window. See crate::bridge's lookup_cached/store_cached.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cached_files (
                remote_path TEXT PRIMARY KEY,
                local_path TEXT NOT NULL,
                remote_modification_time TEXT NOT NULL,
                last_accessed_at INTEGER NOT NULL
            )",
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

    /// The directory `remote_path`'s cached copy belongs in — `root`
    /// itself, mirroring `remote_path`'s parent directory structure.
    /// Creates it if missing. Shared by [`Self::pin`] and the opportunistic
    /// cache (see `crate::bridge`'s `cache_target_dir`/`store_cached_file`)
    /// so a pinned and an opportunistically-cached file live under the same
    /// on-disk layout.
    pub fn target_dir_for(&self, remote_path: &str) -> Result<PathBuf, DriveError> {
        let rel = remote_path.trim_start_matches('/');
        let target_dir = match Path::new(rel).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => self.root.join(parent),
            _ => self.root.clone(),
        };
        std::fs::create_dir_all(&target_dir).map_err(|e| DriveError::Io(e.to_string()))?;
        Ok(target_dir)
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

        let target_dir = self.target_dir_for(remote_path)?;
        cli::download(runner, remote_path, &target_dir)?;

        let rel = remote_path.trim_start_matches('/');
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

    /// Looks up `remote_path` in the opportunistic cache (see issue #60) —
    /// distinct from [`Self::lookup`]'s pin table. Returns the local path
    /// and the remote `modification_time` recorded when it was cached, so
    /// the caller (`crate::bridge`'s `lookup_cached`) can decide whether
    /// it's still fresh — this method itself doesn't re-verify against the
    /// remote, only reads what's on record, same as `cached_stat`/
    /// `cached_listing` (#8) do for metadata. `None` if there's no record,
    /// or the local file is gone (a wiped cache root shouldn't report stale
    /// hits, same guard as [`Self::lookup`]).
    pub fn cached_file(&self, remote_path: &str) -> Result<Option<(PathBuf, String)>, DriveError> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT local_path, remote_modification_time FROM cached_files WHERE remote_path = ?1",
                params![remote_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(row
            .map(|(local, mtime)| (PathBuf::from(local), mtime))
            .filter(|(local, _)| local.is_file()))
    }

    /// Records `remote_path` as opportunistically cached at `local_path`
    /// (upsert) — called after a fresh download/upload, never for a pinned
    /// path (pinning uses the separate `pins` table, see [`Self::pin`]).
    /// `remote_modification_time` is [`NodeEntry::modification_time`] as
    /// known at download/upload time — the caller (`crate::bridge`) already
    /// has it from the `stat`/`finish_download`/`finish_upload` call that
    /// happened right before, so this takes the bare string rather than a
    /// whole `NodeEntry` to construct.
    pub fn store_cached_file(
        &self,
        remote_path: &str,
        local_path: &Path,
        remote_modification_time: &str,
    ) -> Result<(), DriveError> {
        let now = now_unix_secs();
        self.conn
            .execute(
                "INSERT INTO cached_files (remote_path, local_path, remote_modification_time, last_accessed_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(remote_path) DO UPDATE SET
                    local_path = excluded.local_path,
                    remote_modification_time = excluded.remote_modification_time,
                    last_accessed_at = excluded.last_accessed_at",
                params![
                    remote_path,
                    local_path.to_string_lossy(),
                    remote_modification_time,
                    now
                ],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Bumps `remote_path`'s `last_accessed_at` to now — called on a cache
    /// hit that serves the existing local copy without re-downloading, so
    /// the eviction sweep (`daemon/src/cache_eviction.rs`) measures time
    /// since last *use*, not time since it was first cached.
    pub fn touch_cached_file(&self, remote_path: &str) -> Result<(), DriveError> {
        self.conn
            .execute(
                "UPDATE cached_files SET last_accessed_at = ?2 WHERE remote_path = ?1",
                params![remote_path, now_unix_secs()],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Removes `remote_path` from the opportunistic cache: the local file
    /// (best-effort — already gone is not an error) and its record. Used
    /// both by the eviction sweep and by a freshness-check miss (the remote
    /// file changed since it was cached).
    pub fn evict_cached_file(&self, remote_path: &str) -> Result<(), DriveError> {
        if let Some((local_path, _)) = self.cached_file(remote_path)? {
            match std::fs::remove_file(&local_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(DriveError::Io(err.to_string())),
            }
        }
        self.conn
            .execute(
                "DELETE FROM cached_files WHERE remote_path = ?1",
                params![remote_path],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Every opportunistically-cached path whose `last_accessed_at` is at
    /// least `older_than` old — read by the daemon's periodic eviction
    /// sweep. Pinned files never appear here (they live only in `pins`).
    /// `<=`, not `<`: an `older_than` of zero should mean "evict anything
    /// not accessed within this exact instant," i.e. everything — with a
    /// strict `<` a same-second entry could otherwise survive a zero
    /// retention window purely by timing luck.
    pub fn stale_cached_files(&self, older_than: Duration) -> Result<Vec<String>, DriveError> {
        let cutoff = now_unix_secs() - older_than.as_secs() as i64;
        let mut stmt = self
            .conn
            .prepare("SELECT remote_path FROM cached_files WHERE last_accessed_at <= ?1")
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map(params![cutoff], |row| row.get(0))
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| DriveError::Sqlite(e.to_string()))
    }

    /// Whether `remote_path` has *any* locally-available copy — pinned or
    /// opportunistically cached — without checking freshness (used only by
    /// the overlay icon plugin, where a slightly-stale "available locally"
    /// badge is an acceptable cost for not shelling out to `stat` on every
    /// icon repaint; `crate::bridge`'s `lookup_cached` does the real
    /// freshness check for actual `get()` calls).
    pub fn is_available_locally(&self, remote_path: &str) -> Result<bool, DriveError> {
        if self.lookup(remote_path)?.is_some() {
            return Ok(true);
        }
        Ok(self.cached_file(remote_path)?.is_some())
    }

    /// The cached `/photos` node list, if it was fetched within the last
    /// [`PHOTO_TIMELINE_CACHE_TTL`] — `None` on a cold cache or an expired
    /// one, either way meaning the caller should fetch fresh and call
    /// [`Self::store_photo_timeline`].
    pub fn fresh_photo_timeline(&self) -> Result<Option<Vec<NodeEntry>>, DriveError> {
        let cutoff = now_unix_secs() - PHOTO_TIMELINE_CACHE_TTL.as_secs() as i64;
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM photo_timeline_cache WHERE id = 1 AND fetched_at >= ?1",
                params![cutoff],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(|e| DriveError::Parse(e.to_string())))
            .transpose()
    }

    /// Replaces the cached `/photos` node list with `nodes`, timestamped now.
    pub fn store_photo_timeline(&self, nodes: &[NodeEntry]) -> Result<(), DriveError> {
        let payload = serde_json::to_string(nodes).map_err(|e| DriveError::Parse(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO photo_timeline_cache (id, fetched_at, payload) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                    fetched_at = excluded.fetched_at,
                    payload = excluded.payload",
                params![now_unix_secs(), payload],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// The cached [`NodeEntry`] for `path`, if any — no TTL, a hit is always
    /// returned however old (see this module's doc comment for why).
    pub fn cached_stat(&self, path: &str) -> Result<Option<NodeEntry>, DriveError> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT node_json FROM fs_stat_cache WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(|e| DriveError::Parse(e.to_string())))
            .transpose()
    }

    /// Records/replaces `path`'s cached stat result.
    pub fn store_stat(&self, path: &str, node: &NodeEntry) -> Result<(), DriveError> {
        let payload = serde_json::to_string(node).map_err(|e| DriveError::Parse(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO fs_stat_cache (path, node_json, fetched_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                    node_json = excluded.node_json,
                    fetched_at = excluded.fetched_at",
                params![path, payload, now_unix_secs()],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Drops `path`'s cached stat result, if any — called after this app's
    /// own writes (rename/move/trash) so a stale entry doesn't outlive the
    /// change that made it wrong.
    pub fn invalidate_stat(&self, path: &str) -> Result<(), DriveError> {
        self.conn
            .execute("DELETE FROM fs_stat_cache WHERE path = ?1", params![path])
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// The cached child list for `parent_path`, if any — no TTL, same as
    /// [`Self::cached_stat`].
    pub fn cached_listing(&self, parent_path: &str) -> Result<Option<Vec<NodeEntry>>, DriveError> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT children_json FROM fs_listing_cache WHERE parent_path = ?1",
                params![parent_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(|e| DriveError::Parse(e.to_string())))
            .transpose()
    }

    /// Records/replaces `parent_path`'s cached child list, and — since a
    /// full listing already carries every child's complete `NodeEntry` —
    /// also upserts each child individually into `fs_stat_cache`. This
    /// primes instant `stat()`s for anything just listed, which is exactly
    /// the access pattern of e.g. Dolphin's breadcrumb resolving the next
    /// segment down from a folder just browsed into.
    pub fn store_listing(&self, parent_path: &str, nodes: &[NodeEntry]) -> Result<(), DriveError> {
        let payload = serde_json::to_string(nodes).map_err(|e| DriveError::Parse(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO fs_listing_cache (parent_path, children_json, fetched_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(parent_path) DO UPDATE SET
                    children_json = excluded.children_json,
                    fetched_at = excluded.fetched_at",
                params![parent_path, payload, now_unix_secs()],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        let child_path = |node: &NodeEntry| {
            format!(
                "{}/{}",
                parent_path.trim_end_matches('/'),
                node.display_name()
            )
        };
        for node in nodes {
            self.store_stat(&child_path(node), node)?;
        }
        Ok(())
    }

    /// Drops `parent_path`'s cached child list, if any — called after this
    /// app's own writes that change a folder's contents (create/upload/
    /// trash/rename/move).
    pub fn invalidate_listing(&self, parent_path: &str) -> Result<(), DriveError> {
        self.conn
            .execute(
                "DELETE FROM fs_listing_cache WHERE parent_path = ?1",
                params![parent_path],
            )
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        Ok(())
    }

    /// Every path with a cached stat result — read by the sync daemon's
    /// periodic refresh sweep ([`crate::cache`]'s module doc comment).
    pub fn all_cached_stat_paths(&self) -> Result<Vec<String>, DriveError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM fs_stat_cache")
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| DriveError::Sqlite(e.to_string()))
    }

    /// Every parent path with a cached listing — read by the sync daemon's
    /// periodic refresh sweep.
    pub fn all_cached_listing_parents(&self) -> Result<Vec<String>, DriveError> {
        let mut stmt = self
            .conn
            .prepare("SELECT parent_path FROM fs_listing_cache")
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| DriveError::Sqlite(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| DriveError::Sqlite(e.to_string()))
    }
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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

    fn node(uid: &str, name: &str) -> NodeEntry {
        NodeEntry {
            uid: uid.to_string(),
            name: crate::entry::DecryptedField {
                ok: true,
                value: Some(name.to_string()),
            },
            node_type: "file".to_string(),
            media_type: None,
            total_storage_size: Some(123),
            creation_time: "2026-01-01T00:00:00.000Z".to_string(),
            modification_time: "2026-01-01T00:00:00.000Z".to_string(),
            is_shared: false,
            photo: None,
        }
    }

    #[test]
    fn cached_stat_is_none_before_anything_is_stored() {
        let (_dir, cache) = cache();
        assert!(cache.cached_stat("/my-files/a.txt").unwrap().is_none());
    }

    #[test]
    fn store_stat_round_trips_through_cached_stat() {
        let (_dir, cache) = cache();
        let n = node("uid-1", "a.txt");
        cache.store_stat("/my-files/a.txt", &n).unwrap();

        let cached = cache.cached_stat("/my-files/a.txt").unwrap().unwrap();
        assert_eq!(cached.uid, "uid-1");
        assert_eq!(cached.display_name(), "a.txt");
    }

    #[test]
    fn invalidate_stat_clears_a_cached_entry() {
        let (_dir, cache) = cache();
        cache
            .store_stat("/my-files/a.txt", &node("uid-1", "a.txt"))
            .unwrap();

        cache.invalidate_stat("/my-files/a.txt").unwrap();

        assert!(cache.cached_stat("/my-files/a.txt").unwrap().is_none());
    }

    #[test]
    fn store_listing_round_trips_through_cached_listing() {
        let (_dir, cache) = cache();
        let nodes = vec![node("uid-1", "a.txt"), node("uid-2", "b.txt")];

        cache.store_listing("/my-files", &nodes).unwrap();

        let cached = cache.cached_listing("/my-files").unwrap().unwrap();
        let names: Vec<&str> = cached.iter().map(|n| n.display_name()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn store_listing_also_primes_the_stat_cache_for_every_child() {
        let (_dir, cache) = cache();
        let nodes = vec![node("uid-1", "a.txt"), node("uid-2", "sub")];

        cache.store_listing("/my-files", &nodes).unwrap();

        assert_eq!(
            cache.cached_stat("/my-files/a.txt").unwrap().unwrap().uid,
            "uid-1"
        );
        assert_eq!(
            cache.cached_stat("/my-files/sub").unwrap().unwrap().uid,
            "uid-2"
        );
    }

    #[test]
    fn invalidate_listing_clears_a_cached_entry_but_not_its_primed_stats() {
        let (_dir, cache) = cache();
        cache
            .store_listing("/my-files", &[node("uid-1", "a.txt")])
            .unwrap();

        cache.invalidate_listing("/my-files").unwrap();

        assert!(cache.cached_listing("/my-files").unwrap().is_none());
        // Invalidating the listing doesn't imply the individual stats it
        // primed are now wrong too — those are invalidated independently.
        assert!(cache.cached_stat("/my-files/a.txt").unwrap().is_some());
    }

    #[test]
    fn all_cached_stat_paths_lists_every_stat_entry() {
        let (_dir, cache) = cache();
        cache
            .store_stat("/my-files/a.txt", &node("uid-1", "a.txt"))
            .unwrap();
        cache
            .store_stat("/my-files/b.txt", &node("uid-2", "b.txt"))
            .unwrap();

        let mut paths = cache.all_cached_stat_paths().unwrap();
        paths.sort();
        assert_eq!(paths, vec!["/my-files/a.txt", "/my-files/b.txt"]);
    }

    #[test]
    fn all_cached_listing_parents_lists_every_listing_entry() {
        let (_dir, cache) = cache();
        cache
            .store_listing("/my-files", &[node("uid-1", "a.txt")])
            .unwrap();
        cache
            .store_listing("/my-files/sub", &[node("uid-2", "b.txt")])
            .unwrap();

        let mut parents = cache.all_cached_listing_parents().unwrap();
        parents.sort();
        assert_eq!(parents, vec!["/my-files", "/my-files/sub"]);
    }

    #[test]
    fn cached_file_is_none_before_anything_is_stored() {
        let (_dir, cache) = cache();
        assert!(cache.cached_file("/my-files/a.txt").unwrap().is_none());
    }

    #[test]
    fn store_cached_file_round_trips_through_cached_file() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();

        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        let (cached_path, mtime) = cache.cached_file("/my-files/a.txt").unwrap().unwrap();
        assert_eq!(cached_path, local_path);
        assert_eq!(mtime, "2026-01-01T00:00:00.000Z");
    }

    #[test]
    fn cached_file_is_none_when_the_local_copy_is_gone() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        std::fs::remove_file(&local_path).unwrap();

        assert!(cache.cached_file("/my-files/a.txt").unwrap().is_none());
    }

    #[test]
    fn evict_cached_file_removes_the_local_file_and_the_record() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        cache.evict_cached_file("/my-files/a.txt").unwrap();

        assert!(cache.cached_file("/my-files/a.txt").unwrap().is_none());
        assert!(!local_path.exists());
    }

    #[test]
    fn evict_cached_file_is_a_noop_for_something_never_cached() {
        let (_dir, cache) = cache();
        cache.evict_cached_file("/my-files/never.txt").unwrap();
    }

    #[test]
    fn stale_cached_files_finds_entries_older_than_the_cutoff() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/old.txt").unwrap();
        let local_path = target_dir.join("old.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/old.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();
        // Backdate it directly — touch_cached_file/store_cached_file always
        // stamp "now", so an actually-old entry has to be simulated.
        cache
            .conn
            .execute(
                "UPDATE cached_files SET last_accessed_at = ?1 WHERE remote_path = ?2",
                params![now_unix_secs() - 3600, "/my-files/old.txt"],
            )
            .unwrap();

        let stale = cache.stale_cached_files(Duration::from_secs(60)).unwrap();
        assert_eq!(stale, vec!["/my-files/old.txt"]);
    }

    #[test]
    fn stale_cached_files_excludes_recently_accessed_entries() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/fresh.txt").unwrap();
        let local_path = target_dir.join("fresh.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file(
                "/my-files/fresh.txt",
                &local_path,
                "2026-01-01T00:00:00.000Z",
            )
            .unwrap();

        let stale = cache.stale_cached_files(Duration::from_secs(3600)).unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn touch_cached_file_bumps_last_accessed_at() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();
        cache
            .conn
            .execute(
                "UPDATE cached_files SET last_accessed_at = ?1 WHERE remote_path = ?2",
                params![now_unix_secs() - 3600, "/my-files/a.txt"],
            )
            .unwrap();

        cache.touch_cached_file("/my-files/a.txt").unwrap();

        let stale = cache.stale_cached_files(Duration::from_secs(60)).unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn is_available_locally_is_false_for_something_never_pinned_or_cached() {
        let (_dir, cache) = cache();
        assert!(!cache.is_available_locally("/my-files/a.txt").unwrap());
    }

    #[test]
    fn is_available_locally_is_true_for_a_cached_file() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        assert!(cache.is_available_locally("/my-files/a.txt").unwrap());
    }

    #[test]
    fn is_available_locally_is_true_for_a_pinned_file() {
        let (_dir, cache) = cache();
        let runner = DownloadingMockRunner::file(b"hello");
        cache.pin(&runner, "/my-files/a.txt", false).unwrap();

        assert!(cache.is_available_locally("/my-files/a.txt").unwrap());
    }
}
