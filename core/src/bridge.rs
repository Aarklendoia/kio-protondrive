//! The `cxx` FFI boundary consumed by `worker/protondriveworker.cpp`.
//!
//! Kept intentionally thin: this module only translates between
//! [`crate::cli`]/[`crate::entry`] types and flat, cxx-shareable structs, and
//! turns [`crate::cli::DriveError`] into a `Result<_, String>` — cxx surfaces
//! an `Err` as a thrown `rust::Error` on the C++ side, which the KIO worker
//! shim catches and turns into a `KIO::WorkerResult::fail(...)`.

use std::path::Path;

use crate::cli::{self, RealCommandRunner};
use crate::entry::{ListItem, NodeEntry};

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

fn list_dir(path: &str) -> Result<Vec<FfiEntry>, String> {
    let runner = RealCommandRunner;
    let items = cli::list_dir(&runner, path).map_err(|e| e.to_string())?;
    Ok(items.iter().map(item_to_ffi).collect())
}

fn stat_path(path: &str) -> Result<FfiEntry, String> {
    let runner = RealCommandRunner;
    let node = cli::stat_path(&runner, path).map_err(|e| e.to_string())?;
    Ok(node_to_ffi(&node))
}

fn make_dir(parent_path: &str, name: &str) -> Result<FfiEntry, String> {
    let runner = RealCommandRunner;
    let node = cli::create_folder(&runner, parent_path, name).map_err(|e| e.to_string())?;
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
        .map(|_| ())
}

fn trash(path: &str) -> Result<(), String> {
    let runner = RealCommandRunner;
    cli::trash_path(&runner, path).map_err(|e| e.to_string())
}
