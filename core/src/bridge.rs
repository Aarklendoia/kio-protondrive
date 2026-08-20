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
use crate::sharing;
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
        is_shared_by_url: bool,
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

    /// One person with access to a node, or with a pending invitation (see
    /// `crate::sharing::ShareMember`).
    struct FfiShareMember {
        email: String,
        role: String,
        pending: bool,
    }

    /// `has_public_link` is false (and `public_link_url`/`_role`/
    /// `_expiration` all empty) when the node has no active public link —
    /// `FfiPublicLink` itself isn't reused here since cxx shared structs
    /// can't nest an optional one, same reasoning as `FfiPublicLink`'s own
    /// "no `Option<String>` across cxx" convention below.
    struct FfiSharingStatus {
        members: Vec<FfiShareMember>,
        editors_can_share: bool,
        has_public_link: bool,
        public_link_url: String,
        public_link_role: String,
        public_link_expiration: String,
        public_link_downloads: u64,
    }

    /// `expiration` is empty when the link has none — same "no
    /// `Option<String>` across cxx" convention as `FfiEntry`'s `media_type`.
    struct FfiPublicLink {
        url: String,
        role: String,
        expiration: String,
        downloads: u64,
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
        fn lookup_shared(remote_path: &str) -> bool;
        fn list_photos() -> Result<Vec<FfiEntry>>;
        fn list_photos_by_category(category: &str) -> Result<Vec<FfiEntry>>;
        fn stat_photo(name: &str) -> Result<FfiEntry>;
        fn download_photo(name: &str, local_folder: &str) -> Result<String>;
        fn sharing_status(path: &str) -> Result<FfiSharingStatus>;
        fn sharing_invite(path: &str, email: &str, role: &str, message: &str) -> Result<()>;
        fn sharing_remove_member(path: &str, email: &str) -> Result<()>;
        fn sharing_set_link(
            path: &str,
            role: &str,
            password: &str,
            expiration: &str,
        ) -> Result<FfiPublicLink>;
        fn sharing_remove_link(path: &str) -> Result<()>;
    }
}

use ffi::{FfiEntry, FfiPublicLink, FfiShareMember, FfiSharingStatus, FfiTransferPoll};

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
        is_shared_by_url: node.is_shared_by_url,
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
            is_shared_by_url: false,
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

/// Whether `remote_path` has a member (or pending invitation) or an active
/// public link, for `worker/overlayplugin.cpp`'s "shared" badge —
/// deliberately reads only `fs_stat_cache` (no live CLI call: this is
/// called once per visible icon on every repaint, and a live `filesystem
/// info` round-trip per icon would be far too slow, same reasoning as
/// [`is_available_locally`]'s own cache-only stance). `list_dir`/
/// `stat_path` already populate this cache with every node's real
/// `isShared`/`isSharedByUrl` as a side effect of ordinary
/// browsing, so this only ever sees stale data for a path never listed or
/// stat'd yet (false, same as "not shared" until proven otherwise) or one
/// whose sharing changed through this dialog since (see `refresh_stat_cache`,
/// called by every mutating `sharing_*` function below to keep this fresh).
fn lookup_shared(remote_path: &str) -> bool {
    open_cache()
        .ok()
        .and_then(|cache| cache.cached_stat(remote_path).ok().flatten())
        .is_some_and(|node| node.is_shared || node.is_shared_by_url)
}

/// Re-fetches `path`'s live stat and overwrites its `fs_stat_cache` entry —
/// called after every mutating `sharing_*` call below so [`lookup_shared`]
/// (and any other `fs_stat_cache` reader) reflects the change immediately
/// instead of only after the next unrelated browse/stat of the same path.
/// Mirrors `daemon::fs_refresh::refresh_all`'s own stat-refresh loop,
/// including its `NotFound` handling: a sharing action racing a concurrent
/// delete/move elsewhere shouldn't leave a stale (possibly `is_shared:
/// true`) cache entry outliving the node itself. Any other error is
/// best-effort, same stance as `refresh_all` — leaves the previous cached
/// value in place a little longer rather than surfacing from what's
/// otherwise a successful sharing action.
fn refresh_stat_cache(path: &str) {
    let Ok(cache) = open_cache() else {
        return;
    };
    let runner = RealCommandRunner;
    match cli::stat_path(&runner, path) {
        Ok(node) => {
            let _ = cache.store_stat(path, &node);
        }
        Err(cli::DriveError::NotFound(_)) => {
            let _ = cache.invalidate_stat(path);
        }
        Err(_) => {}
    }
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

/// `category` is one of `photos::PhotoCategory::ALL`'s slugs (e.g.
/// `"videos"`) — the worker only ever calls this with one it already
/// validated against that same list, so an unrecognized slug here is a
/// worker-side bug, not a reachable user-facing error.
fn list_photos_by_category(category: &str) -> Result<Vec<FfiEntry>, String> {
    let category = photos::PhotoCategory::from_slug(category)
        .ok_or_else(|| format!("unknown photo category: {category}"))?;
    let filtered = photos::filter_by_category(&cached_photos()?, category);
    Ok(filtered.iter().map(photo_to_ffi).collect())
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

fn share_member_to_ffi(member: &sharing::ShareMember) -> FfiShareMember {
    FfiShareMember {
        email: member.email.clone(),
        role: member.role.clone(),
        pending: member.pending,
    }
}

fn sharing_status(path: &str) -> Result<FfiSharingStatus, String> {
    let runner = RealCommandRunner;
    let status = sharing::status(&runner, path).map_err(|e| e.to_string())?;
    let (
        has_public_link,
        public_link_url,
        public_link_role,
        public_link_expiration,
        public_link_downloads,
    ) = match status.public_link {
        Some(link) => (
            true,
            link.url,
            link.role,
            link.expiration_time.unwrap_or_default(),
            link.number_of_initialized_downloads,
        ),
        None => (false, String::new(), String::new(), String::new(), 0),
    };
    Ok(FfiSharingStatus {
        members: status.members.iter().map(share_member_to_ffi).collect(),
        editors_can_share: status.editors_can_share,
        has_public_link,
        public_link_url,
        public_link_role,
        public_link_expiration,
        public_link_downloads,
    })
}

fn sharing_invite(path: &str, email: &str, role: &str, message: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    sharing::invite(&runner, path, email, role, message).map_err(|e| e.to_string())?;
    refresh_stat_cache(path);
    Ok(())
}

fn sharing_remove_member(path: &str, email: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    sharing::remove_member(&runner, path, email).map_err(|e| e.to_string())?;
    refresh_stat_cache(path);
    Ok(())
}

fn sharing_set_link(
    path: &str,
    role: &str,
    password: &str,
    expiration: &str,
) -> Result<FfiPublicLink, String> {
    let runner = RealCommandRunner;
    let link =
        sharing::set_link(&runner, path, role, password, expiration).map_err(|e| e.to_string())?;
    refresh_stat_cache(path);
    Ok(FfiPublicLink {
        url: link.url,
        role: link.role,
        expiration: link.expiration_time.unwrap_or_default(),
        downloads: link.number_of_initialized_downloads,
    })
}

fn sharing_remove_link(path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    sharing::remove_link(&runner, path).map_err(|e| e.to_string())?;
    refresh_stat_cache(path);
    Ok(())
}
