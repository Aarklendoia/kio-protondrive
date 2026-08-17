//! Phase 1 sync daemon: one-way local -> Proton Drive upload.
//!
//! Watches a single configured local folder (see [`config`]) and uploads
//! new/changed files, using a SQLite journal (see [`journal`]) purely to
//! avoid re-uploading unchanged files. Renames/moves within the watched
//! folder are propagated as renames/moves on Drive rather than uploaded as
//! new files (see [`sync::handle_rename`]). Drive -> local download,
//! local-delete propagation, and conflict resolution are later phases —
//! see docs/DESIGN.md.

mod config;
mod error;
mod journal;
mod notification;
mod sync;
mod version_check;
mod watcher;

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use protondrive_core::cli::RealCommandRunner;

use config::Config;
use journal::Journal;
use watcher::WatchEvent;

/// How often to ask the installed `proton-drive` CLI whether a newer
/// release exists (see [`version_check`]) — infrequent by design, this is a
/// convenience nudge, not something latency-sensitive.
const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Logs one clear, actionable line and fires a desktop notification — but
/// only on the falling edge (the first failure after things were working),
/// so a stuck-unauthenticated daemon doesn't spam either every cycle.
/// `auth_notified` flips back to false as soon as a sync succeeds again, so
/// a *later* re-expiry (e.g. days from now) still gets a fresh notification.
fn report_authentication_failure(auth_notified: &mut bool) {
    if *auth_notified {
        return;
    }
    log::error!(
        "Proton Drive session missing or expired — run `proton-drive auth login`, then Proton \
         Drive sync will resume automatically (no need to restart this service)"
    );
    notification::auth_required();
    *auth_notified = true;
}

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
    let mut auth_notified = false;

    log::info!(
        "reconciling {} -> {}",
        config.local_path.display(),
        config.remote_path
    );
    match sync::reconcile(&runner, &journal, &config) {
        Ok(()) => auth_notified = false,
        Err(err) if err.is_authentication_error() => {
            report_authentication_failure(&mut auth_notified)
        }
        Err(err) => log::error!("initial reconcile failed: {err}"),
    }

    let (_watcher, events) = match watcher::watch(&config.local_path) {
        Ok(handle) => handle,
        Err(err) => {
            log::error!("failed to watch {}: {err}", config.local_path.display());
            std::process::exit(1);
        }
    };

    log::info!("watching {} for changes", config.local_path.display());
    // Checked once immediately here (same shape as the reconcile() call
    // above), then every VERSION_CHECK_INTERVAL from inside the loop below.
    // Note this must NOT be `Instant::now()` followed by relying on the
    // loop's own elapsed() >= INTERVAL check to fire it "right away": a
    // freshly-started Instant has elapsed() ~0, which makes the *wait*
    // computed below ~INTERVAL (not ~0) — that would silently delay the
    // first check by a full day instead of running it at startup.
    let mut cli_update_notified: Option<String> = None;
    version_check::check(&runner, &mut cli_update_notified);
    let mut last_version_check = Instant::now();
    loop {
        let wait = VERSION_CHECK_INTERVAL.saturating_sub(last_version_check.elapsed());
        match events.recv_timeout(wait) {
            Ok(batch) => {
                for event in batch {
                    let label = match &event {
                        WatchEvent::Changed(path) => path.display().to_string(),
                        WatchEvent::Renamed { from, to } => {
                            format!("{} -> {}", from.display(), to.display())
                        }
                    };
                    let result = match &event {
                        WatchEvent::Changed(path) => {
                            sync::upload_if_needed(&runner, &journal, &config, path)
                        }
                        WatchEvent::Renamed { from, to } => {
                            sync::handle_rename(&runner, &journal, &config, from, to)
                        }
                    };
                    match result {
                        Ok(()) => auth_notified = false,
                        Err(err) if err.is_authentication_error() => {
                            report_authentication_failure(&mut auth_notified);
                            // The rest of this batch would fail identically —
                            // skip straight to the next debounced batch
                            // instead of burning a CLI call per remaining
                            // event.
                            break;
                        }
                        Err(err) => {
                            log::warn!("failed to sync {label}: {err} (will retry next cycle)")
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The watcher's sender half is gone — nothing more will ever
            // arrive, so there's nothing left for this loop to do.
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if last_version_check.elapsed() >= VERSION_CHECK_INTERVAL {
            version_check::check(&runner, &mut cli_update_notified);
            last_version_check = Instant::now();
        }
    }
}
