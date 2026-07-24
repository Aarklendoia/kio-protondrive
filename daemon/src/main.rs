//! Phase 1 sync daemon: one-way local -> Proton Drive upload.
//!
//! Watches a single configured local folder (see [`config`]) and uploads
//! new/changed files, using a SQLite journal (see [`journal`]) purely to
//! avoid re-uploading unchanged files. Drive -> local download, local-delete
//! propagation, and conflict resolution are later phases — see
//! docs/DESIGN.md.

mod config;
mod error;
mod journal;
mod sync;
mod watcher;

use protondrive_core::cli::RealCommandRunner;

use config::Config;
use journal::Journal;

fn main() {
    env_logger::init();

    let config_path = Config::default_path();
    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            log::error!(
                "failed to load config from {}: {err}",
                config_path.display()
            );
            std::process::exit(1);
        }
    };

    let journal = match Journal::open(&Journal::default_path()) {
        Ok(journal) => journal,
        Err(err) => {
            log::error!("failed to open journal: {err}");
            std::process::exit(1);
        }
    };

    let runner = RealCommandRunner;

    log::info!(
        "reconciling {} -> {}",
        config.local_path.display(),
        config.remote_path
    );
    if let Err(err) = sync::reconcile(&runner, &journal, &config) {
        log::error!("initial reconcile failed: {err}");
    }

    let (_watcher, events) = match watcher::watch(&config.local_path) {
        Ok(handle) => handle,
        Err(err) => {
            log::error!("failed to watch {}: {err}", config.local_path.display());
            std::process::exit(1);
        }
    };

    log::info!("watching {} for changes", config.local_path.display());
    for paths in events {
        for path in paths {
            if let Err(err) = sync::upload_if_needed(&runner, &journal, &config, &path) {
                log::warn!(
                    "failed to sync {}: {err} (will retry next cycle)",
                    path.display()
                );
            }
        }
    }
}
