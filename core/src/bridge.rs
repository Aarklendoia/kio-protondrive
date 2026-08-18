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

    extern "Rust" {
        fn list_dir(path: &str) -> Result<Vec<FfiEntry>>;
        fn stat_path(path: &str) -> Result<FfiEntry>;
        fn make_dir(parent_path: &str, name: &str) -> Result<FfiEntry>;
        fn download_to(remote_path: &str, local_folder: &str) -> Result<()>;
        fn upload_from(local_path: &str, parent_path: &str) -> Result<()>;
        fn trash(path: &str) -> Result<()>;
        fn rename_or_move(old_path: &str, new_path: &str) -> Result<()>;
        fn lookup_pin(remote_path: &str) -> Result<String>;
        fn unpin_path(remote_path: &str, force: bool) -> Result<()>;
        fn list_photos() -> Result<Vec<FfiEntry>>;
        fn stat_photo(name: &str) -> Result<FfiEntry>;
        fn download_photo(name: &str, local_folder: &str) -> Result<String>;
    }
}

use ffi::FfiEntry;

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

fn download_to(remote_path: &str, local_folder: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::download(&runner, remote_path, Path::new(local_folder))
        .map_err(|e| e.to_string())
        .map(|_| ())
}

fn upload_from(local_path: &str, parent_path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::upload(&runner, Path::new(local_path), parent_path)
        .map_err(|e| e.to_string())
        .map(|_| ())?;
    if let Ok(cache) = open_cache() {
        let _ = cache.invalidate_listing(parent_path);
    }
    Ok(())
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
