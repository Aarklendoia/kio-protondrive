//! Library surface for the sync daemon.
//!
//! `main.rs` (the actual `kio-protondrive-daemon` binary) is a thin runner
//! over these same modules — they live here, not directly in the binary,
//! so `wizard/` can depend on this crate too and reuse [`config::Config`]
//! (to write `daemon.toml` in the exact shape the daemon reads) and
//! [`error::DaemonError`], instead of duplicating either.

pub mod config;
pub mod control;
pub mod error;
pub mod notification;
pub mod sync;
pub mod watcher;
