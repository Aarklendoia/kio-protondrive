//! The `cxx` FFI boundary consumed by `worker/protondriveworker.cpp`.
//!
//! Kept intentionally thin: this module only translates between
//! [`crate::cli`]/[`crate::entry`] types and flat, cxx-shareable structs, and
//! turns [`crate::cli::DriveError`] into a `Result<_, String>` — cxx surfaces
//! an `Err` as a thrown `rust::Error` on the C++ side, which the KIO worker
//! shim catches and turns into a `KIO::WorkerResult::fail(...)`.

use std::path::Path;

use crate::cache::Cache;
use crate::cli::{self, RealCommandRunner};
use crate::entry::{ListItem, NodeEntry};
use crate::photos;
use crate::transfer::{Direction, TransferPoll};

#[cxx::bridge(namespace = "protondrive")]
mod ffi {
    /// Flattened view of a Proton Drive node (or virtual root section) for
    /// consumption by the C++ `KIO::WorkerBase` implementation. `media_type`
    /// and `modification_time` are empty strings when unknown/not applicable
    /// (e.g. a folder has no media type) rather than an `Option`, since cxx
    /// shared structs can't hold `Option<String>` directly.
    struct FfiEntry {
        name: String,
        is_folder: bool,
        media_type: String,
        size: u64,
        creation_time: String,
        modification_time: String,
    }

    /// Result of one poll of an in-flight [`TransferHandle`]. `ok`/`error`
    /// are only meaningful when `done` is true — cxx has no `Option<T>` for
    /// arbitrary shared-struct fields, so `done` itself is the discriminant.
    /// `processed_bytes` is always meaningful: an elapsed-time-based
    /// estimate while running (see `crate::transfer`'s doc comment for why
    /// it's an estimate, not a real byte count), and the real total once
    /// `done && ok`.
    struct FfiTransferPoll {
        done: bool,
        ok: bool,
        error: String,
        processed_bytes: u64,
    }

    extern "Rust" {
        type TransferHandle;

        fn list_dir(path: &str) -> Result<Vec<FfiEntry>>;
        fn stat_path(path: &str) -> Result<FfiEntry>;
        fn make_dir(parent_path: &str, name: &str) -> Result<FfiEntry>;
        fn start_download(
            remote_path: &str,
            local_folder: &str,
            total_bytes: u64,
        ) -> Result<Box<TransferHandle>>;
        fn start_upload(
            local_path: &str,
            parent_path: &str,
            total_bytes: u64,
        ) -> Result<Box<TransferHandle>>;
        fn poll_transfer(handle: &mut TransferHandle) -> FfiTransferPoll;
        fn cancel_transfer(handle: &mut TransferHandle);
        fn trash(path: &str) -> Result<()>;
        fn restore_path(remote_path: &str) -> Result<()>;
        fn permanently_delete_path(remote_path: &str) -> Result<()>;
        fn empty_trash() -> Result<()>;
        fn rename_or_move(old_path: &str, new_path: &str) -> Result<()>;
        fn lookup_pin(remote_path: &str) -> Result<String>;
        fn unpin_path(remote_path: &str, force: bool) -> Result<()>;
        fn lookup_cached(remote_path: &str) -> Result<String>;
        fn cache_target_dir(remote_path: &str) -> Result<String>;
        fn store_cached(remote_path: &str, local_path: &str, modification_time: &str)
            -> Result<()>;
        fn is_available_locally(remote_path: &str) -> bool;
        fn list_photos() -> Result<Vec<FfiEntry>>;
        fn stat_photo(name: &str) -> Result<FfiEntry>;
        fn download_photo(name: &str, local_folder: &str) -> Result<String>;
    }
}

use ffi::{FfiEntry, FfiTransferPoll};

/// cxx-exposed wrapper around `crate::transfer::TransferHandle` — adds just
/// enough context (the path, kept only for `finish_download`/
/// `finish_upload`'s error messages) that `poll_transfer` below can validate
/// a finished transfer's `CommandOutput` the same way the old blocking
/// `download()`/`upload()` did, without `crate::transfer` itself needing to
/// know anything about downloads/uploads/paths.
pub struct TransferHandle {
    inner: crate::transfer::TransferHandle,
    path: String,
}

fn node_to_ffi(node: &NodeEntry) -> FfiEntry {
    FfiEntry {
        name: node.display_name().to_string(),
        is_folder: node.is_folder(),
        media_type: node.media_type.clone().unwrap_or_default(),
        size: node.total_storage_size.unwrap_or(0),
        creation_time: node.creation_time.clone(),
        modification_time: node.modification_time.clone(),
    }
}

fn item_to_ffi(item: &ListItem) -> FfiEntry {
    match item {
        ListItem::Node(node) => node_to_ffi(node),
        ListItem::Section(section) => FfiEntry {
            name: section.display_name().to_string(),
            is_folder: true,
            media_type: String::new(),
            size: 0,
            creation_time: String::new(),
            modification_time: String::new(),
        },
    }
}

fn open_cache() -> Result<Cache, String> {
    Cache::open(&Cache::default_db_path(), &Cache::default_root()).map_err(|e| e.to_string())
}

/// The virtual root (`/`) is never cached (see `crate::cache`'s module doc
/// comment): its entries are Proton Drive's fixed sections, not real nodes
/// (`ListItem::Section`, which doesn't even round-trip through JSON the way
/// [`NodeEntry`] does), and it's already small/fast — caching it would add
/// real-path-cache complexity for close to no benefit.
fn list_dir(path: &str) -> Result<Vec<FfiEntry>, String> {
    let runner = RealCommandRunner;
    if path == "/" {
        let items = cli::list_dir(&runner, path).map_err(|e| e.to_string())?;
        return Ok(items.iter().map(item_to_ffi).collect());
    }

    // Best-effort accelerator, same stance as lookup_pin: any cache-open
    // failure just means always-live browsing, never a hard failure.
    if let Ok(cache) = open_cache() {
        if let Ok(Some(nodes)) = cache.cached_listing(path) {
            return Ok(nodes.iter().map(node_to_ffi).collect());
        }
        let items = cli::list_dir(&runner, path).map_err(|e| e.to_string())?;
        let nodes: Vec<NodeEntry> = items
            .into_iter()
            .filter_map(|item| match item {
                ListItem::Node(node) => Some(node),
                // A non-root path only ever returns real nodes (see
                // entry.rs's ListItem doc comment) — a Section here would be
                // unexpected, not worth failing the whole listing over.
                ListItem::Section(_) => None,
            })
            .collect();
        let _ = cache.store_listing(path, &nodes);
        return Ok(nodes.iter().map(node_to_ffi).collect());
    }

    let items = cli::list_dir(&runner, path).map_err(|e| e.to_string())?;
    Ok(items.iter().map(item_to_ffi).collect())
}

fn stat_path(path: &str) -> Result<FfiEntry, String> {
    let runner = RealCommandRunner;
    if let Ok(cache) = open_cache() {
        if let Ok(Some(node)) = cache.cached_stat(path) {
            return Ok(node_to_ffi(&node));
        }
        let node = cli::stat_path(&runner, path).map_err(|e| e.to_string())?;
        let _ = cache.store_stat(path, &node);
        return Ok(node_to_ffi(&node));
    }
    let node = cli::stat_path(&runner, path).map_err(|e| e.to_string())?;
    Ok(node_to_ffi(&node))
}

fn make_dir(parent_path: &str, name: &str) -> Result<FfiEntry, String> {
    let runner = RealCommandRunner;
    let node = cli::create_folder(&runner, parent_path, name).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_listing(parent_path);
        let child_path = format!("{}/{name}", parent_path.trim_end_matches('/'));
        let _ = cache.store_stat(&child_path, &node);
    }
    Ok(node_to_ffi(&node))
}

/// Starts a cancellable, progress-estimating download — see
/// `crate::transfer`'s module doc comment for why this can't offer real
/// byte-accurate progress. The parent path's listing isn't invalidated here
/// (unlike `start_upload`): a download never changes anything remote.
fn start_download(
    remote_path: &str,
    local_folder: &str,
    total_bytes: u64,
) -> Result<Box<TransferHandle>, String> {
    let args = cli::download_args(remote_path, Path::new(local_folder));
    let inner = crate::transfer::TransferHandle::start(Direction::Download, args, total_bytes)
        .map_err(|e| e.to_string())?;
    Ok(Box::new(TransferHandle {
        inner,
        path: remote_path.to_string(),
    }))
}

/// Starts a cancellable, progress-estimating upload. Cache invalidation
/// happens once `poll_transfer` reports success (mirroring `upload_from`'s
/// old placement right after the CLI call succeeded), not here at start —
/// nothing has actually changed remotely yet.
fn start_upload(
    local_path: &str,
    parent_path: &str,
    total_bytes: u64,
) -> Result<Box<TransferHandle>, String> {
    let args = cli::upload_args(Path::new(local_path), parent_path);
    let inner = crate::transfer::TransferHandle::start(Direction::Upload, args, total_bytes)
        .map_err(|e| e.to_string())?;
    Ok(Box::new(TransferHandle {
        inner,
        path: parent_path.to_string(),
    }))
}

/// Validates a just-finished `CommandOutput` the same way the old blocking
/// `download()`/`upload()` did (`ensure_success` + JSON parse +
/// `ensure_no_failures`) — which of the two to apply is read back off the
/// handle's own `direction()` rather than passed in again, so this one
/// function serves both `poll_transfer` call sites.
fn poll_transfer(handle: &mut TransferHandle) -> FfiTransferPoll {
    match handle.inner.poll() {
        TransferPoll::Pending { estimated_bytes } => FfiTransferPoll {
            done: false,
            ok: false,
            error: String::new(),
            processed_bytes: estimated_bytes,
        },
        TransferPoll::Done(Ok(out)) => {
            let result = match handle.inner.direction() {
                Direction::Download => cli::finish_download(&handle.path, out),
                Direction::Upload => cli::finish_upload(&handle.path, out),
            };
            match result {
                Ok(summary) => {
                    if handle.inner.direction() == Direction::Upload {
                        if let Ok(cache) = open_cache() {
                            let _ = cache.invalidate_listing(&handle.path);
                        }
                    }
                    FfiTransferPoll {
                        done: true,
                        ok: true,
                        error: String::new(),
                        processed_bytes: summary.transferred_bytes,
                    }
                }
                Err(err) => FfiTransferPoll {
                    done: true,
                    ok: false,
                    error: err.to_string(),
                    processed_bytes: 0,
                },
            }
        }
        TransferPoll::Done(Err(err)) => FfiTransferPoll {
            done: true,
            ok: false,
            error: err.to_string(),
            processed_bytes: 0,
        },
    }
}

fn cancel_transfer(handle: &mut TransferHandle) {
    handle.inner.cancel();
}

fn trash(path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::trash_path(&runner, path).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_stat(path);
        if let Some((parent, _)) = path.rsplit_once('/') {
            let parent = if parent.is_empty() { "/" } else { parent };
            let _ = cache.invalidate_listing(parent);
        }
    }
    Ok(())
}

/// The item's actual destination (wherever it lived before being trashed)
/// isn't known here — the CLI doesn't report it — so only `/trash`'s own
/// listing is invalidated; the destination folder's listing stays stale
/// until #8's periodic sweep or a direct access, same tradeoff already
/// accepted everywhere else in this cache.
fn restore_path(remote_path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::restore_path(&runner, remote_path).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_stat(remote_path);
        let _ = cache.invalidate_listing("/trash");
    }
    Ok(())
}

fn permanently_delete_path(remote_path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::permanently_delete_path(&runner, remote_path).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_stat(remote_path);
        let _ = cache.invalidate_listing("/trash");
    }
    Ok(())
}

fn empty_trash() -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::empty_trash(&runner).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_listing("/trash");
    }
    Ok(())
}

fn rename_or_move(old_path: &str, new_path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::rename_or_move(&runner, old_path, new_path).map_err(|e| e.to_string())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_stat(old_path);
        let _ = cache.invalidate_stat(new_path);
        for path in [old_path, new_path] {
            if let Some((parent, _)) = path.rsplit_once('/') {
                let parent = if parent.is_empty() { "/" } else { parent };
                let _ = cache.invalidate_listing(parent);
            }
        }
    }
    Ok(())
}

/// Empty string means "not pinned" — same "no `Option<String>` across cxx"
/// convention as [`FfiEntry`]'s `media_type`/`modification_time`.
fn lookup_pin(remote_path: &str) -> Result<String, String> {
    let cache = open_cache()?;
    let local = cache.lookup(remote_path).map_err(|e| e.to_string())?;
    Ok(local
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// Called from `del()` after a successful trash — with `force: true`,
/// always dropping the local cache copy regardless of unsynced edits, since
/// the remote it would otherwise be uploaded to no longer exists. A no-op
/// (via [`crate::cache::Cache::unpin`]) if `remote_path` wasn't pinned.
fn unpin_path(remote_path: &str, force: bool) -> Result<(), String> {
    let cache = open_cache()?;
    cache.unpin(remote_path, force).map_err(|e| e.to_string())
}

/// Opportunistic-cache read (issue #60) — empty string means "miss", same
/// convention as [`lookup_pin`]. Unlike `lookup_pin`, a hit here is
/// re-verified against the remote's current `modification_time` before
/// being trusted (see `crate::cache`'s module doc comment for why: unlike
/// an explicit pin, a file can change elsewhere without the user pinning
/// anything to be told about it). A stale hit is evicted here rather than
/// left for the daemon's sweep to eventually catch, so the very next
/// download replaces it immediately instead of orphaning the old copy.
fn lookup_cached(remote_path: &str) -> Result<String, String> {
    let cache = open_cache()?;
    let Some((local_path, cached_mtime)) =
        cache.cached_file(remote_path).map_err(|e| e.to_string())?
    else {
        return Ok(String::new());
    };
    let current = stat_path(remote_path)?;
    if current.modification_time == cached_mtime {
        let _ = cache.touch_cached_file(remote_path);
        Ok(local_path.to_string_lossy().into_owned())
    } else {
        let _ = cache.evict_cached_file(remote_path);
        Ok(String::new())
    }
}

/// Where a fresh download/upload of `remote_path` should land on disk, so
/// it survives past the KIO call that created it — the opportunistic-cache
/// counterpart to [`crate::cache::Cache::pin`]'s own target directory,
/// reusing the exact same mirrored layout via
/// [`crate::cache::Cache::target_dir_for`] so a pinned and an
/// opportunistically-cached copy of related files share one on-disk tree.
fn cache_target_dir(remote_path: &str) -> Result<String, String> {
    let cache = open_cache()?;
    let dir = cache
        .target_dir_for(remote_path)
        .map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Records a just-finished download/upload in the opportunistic cache.
/// `modification_time` is whatever the worker already had on hand from a
/// `stat`/transfer-completion call — see `worker/protondriveworker.cpp`'s
/// `get()`/`put()` for exactly which call each reuses instead of paying for
/// an extra one just for this.
fn store_cached(
    remote_path: &str,
    local_path: &str,
    modification_time: &str,
) -> Result<(), String> {
    let cache = open_cache()?;
    cache
        .store_cached_file(remote_path, Path::new(local_path), modification_time)
        .map_err(|e| e.to_string())
}

/// Whether `remote_path` has *any* locally-available copy — pinned or
/// opportunistically cached — for `worker/overlayplugin.cpp`'s "available
/// locally" badge (shared by both states; pin gets its own additional
/// badge on top, checked separately via [`lookup_pin`]). Best-effort, same
/// stance as every other lookup this plugin does: a cache-open failure just
/// means no badge, not an error worth surfacing from an icon-decoration
/// hook.
fn is_available_locally(remote_path: &str) -> bool {
    open_cache()
        .and_then(|cache| {
            cache
                .is_available_locally(remote_path)
                .map_err(|e| e.to_string())
        })
        .unwrap_or(false)
}

fn photo_to_ffi(photo: &photos::Photo) -> FfiEntry {
    let mut entry = node_to_ffi(&photo.node);
    entry.name = photo.display_name.clone();
    entry
}

/// `photo timeline -d` (see `photos::list_photos`) has no way to fetch
/// details for a single photo, and can take well over a minute for a large
/// library (confirmed live: ~80s for ~12k photos) — without memoizing it,
/// every `stat_photo`/`download_photo` call (i.e. every thumbnail Dolphin
/// generates while browsing `/photos`) would independently re-pay that
/// cost. Uses `crate::cache::Cache`'s on-disk `photo_timeline_cache` table
/// (shared/lock-safe across processes) rather than an in-process cache —
/// confirmed live that Dolphin's `kio-fuse` mount spawns short-lived,
/// disposable worker processes to serve `/photos` thumbnail requests, each
/// of which would otherwise start with a cold, empty cache of its own and
/// get killed by an impatient caller before finishing, in a loop. Kept out
/// of `photos::list_photos`/`photos::disambiguate` themselves so that
/// module's logic stays unit-testable without any I/O of its own.
fn cached_photos() -> Result<Vec<photos::Photo>, String> {
    let cache = open_cache()?;
    if let Some(nodes) = cache.fresh_photo_timeline().map_err(|e| e.to_string())? {
        return Ok(photos::disambiguate(nodes));
    }
    let runner = RealCommandRunner;
    let nodes = cli::photo_timeline(&runner).map_err(|e| e.to_string())?;
    cache
        .store_photo_timeline(&nodes)
        .map_err(|e| e.to_string())?;
    Ok(photos::disambiguate(nodes))
}

fn cached_find_photo(name: &str) -> Result<photos::Photo, String> {
    cached_photos()?
        .into_iter()
        .find(|photo| photo.display_name == name)
        // Same message shape as photos::find_photo's own DriveError::NotFound,
        // so resultFromRustError on the C++ side still maps it the same way.
        .ok_or_else(|| cli::DriveError::NotFound(format!("/photos/{name}")).to_string())
}

fn list_photos() -> Result<Vec<FfiEntry>, String> {
    Ok(cached_photos()?.iter().map(photo_to_ffi).collect())
}

fn stat_photo(name: &str) -> Result<FfiEntry, String> {
    let photo = cached_find_photo(name)?;
    Ok(photo_to_ffi(&photo))
}

/// Returns the filename the CLI actually wrote under `local_folder` — not
/// guaranteed to equal `name` (see `photos::list_photos`'s disambiguation
/// suffix for same-named photos), so the caller must read it back rather
/// than assume the two agree.
fn download_photo(name: &str, local_folder: &str) -> Result<String, String> {
    let photo = cached_find_photo(name)?;
    let runner = RealCommandRunner;
    cli::photo_download(&runner, &photo.node.uid, Path::new(local_folder))
        .map_err(|e| e.to_string())?;
    let mut entries = std::fs::read_dir(local_folder).map_err(|e| e.to_string())?;
    let downloaded = entries
        .next()
        .ok_or_else(|| "photo download produced no local file".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(downloaded.file_name().to_string_lossy().into_owned())
}
