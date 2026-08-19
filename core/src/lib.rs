//! Core logic for the `protondrive://` KIO worker.
//!
//! This crate contains everything that doesn't strictly require linking
//! against `KF6::KIOCore`: it shells out to the official `proton-drive` CLI,
//! parses its JSON output, and exposes a small [`cxx`] bridge
//! ([`bridge`]) consumed by the C++ `KIO::WorkerBase` shim in `worker/`.

pub mod bridge;
pub mod cache;
pub mod cli;
pub mod cli_update;
pub mod entry;
pub mod local_ctrl;
pub mod photos;
pub mod sharing;
pub mod transfer;
