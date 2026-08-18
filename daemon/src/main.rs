//! Sync daemon for **pinned** files (see issue #30 and
//! `protondrive_core::cache`): watches the local pin cache directory and
//! uploads changes to whichever pinned files were edited, using the cache
//! index purely to know each local file's remote destination and avoid
//! re-uploading unchanged files. Renames/moves within the cache directory
//! are propagated as renames/moves on Drive rather than uploaded as new
//! files (see [`sync::handle_rename`]).
//!
//! Also runs as a one-shot CLI client in `pin`/`unpin` mode — see
//! [`control::run_client`] — which is what the Dolphin ServiceMenu action
//! actually invokes (talking to the *already-running* instance of this
//! same binary over its local control server, started by the branch below).

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use protondrive_core::cache::Cache;
use protondrive_core::cli::RealCommandRunner;

use kio_protondrive_daemon::config::Config;
use kio_protondrive_daemon::watcher::{self, WatchEvent};
use kio_protondrive_daemon::{
    cache_eviction, control, fs_refresh, notification, sync, version_check,
};

/// How often to ask the installed `proton-drive` CLI whether a newer
/// release exists (see [`version_check`]) — infrequent by design, this is a
/// convenience nudge, not something latency-sensitive.
const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How often to refresh `core::cache`'s permanent filesystem stat/listing
/// cache (see [`fs_refresh`]) — a tradeoff between staying reasonably fresh
/// and not hammering the CLI: each cached path costs its own ~1-4s CLI call,
/// sequential, so a large cache does take a while to fully sweep.
const FS_CACHE_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How often to sweep `core::cache`'s opportunistic file cache for entries
/// past the configured retention window (see [`cache_eviction`], issue
/// #60) — daily is plenty since the retention window itself is measured in
/// days, unlike the much shorter-lived data the other two intervals above
/// guard.
const CACHE_EVICTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Logs one clear, actionable line and fires a desktop notification — but
/// only on the falling edge (the first failure after things were working),
/// so a stuck-unauthenticated daemon doesn't spam either every cycle.
/// `auth_notified` flips back to false as soon as a sync succeeds again, so
/// a *later* re-expiry (e.g. days from now) still gets a fresh notification.
/// Also what launches the setup wizard, since "not authenticated" is the
/// only thing left that actually blocks this daemon from doing its job —
/// there's no required config to be missing anymore (see `config`'s doc
/// comment).
fn report_authentication_failure(auth_notified: &mut bool) {
    if *auth_notified {
        return;
    }
    log::error!(
        "Proton Drive session missing or expired — run `proton-drive auth login`, then Proton \
         Drive sync will resume automatically (no need to restart this service)"
    );
    notification::auth_required();
    // Best-effort: kio-protondrive-wizard is a Recommends, not a hard
    // Depends, so it may not be installed — that's fine, the user just has
    // to run `proton-drive auth login` themselves per the notification.
    if let Err(err) = std::process::Command::new("kio-protondrive-wizard").spawn() {
        log::debug!("could not launch the setup wizard (not installed?): {err}");
    }
    *auth_notified = true;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && (args[1] == "pin" || args[1] == "unpin") {
        run_pin_client(&args[1], &args[2]);
        return;
    }

    env_logger::init();
    notification::init();

    let config_path = Config::default_path();
    let config = match Config::load_or_default(&config_path) {
        Ok(config) => config,
        Err(err) => {
            log::error!(
                "failed to load config from {}: {err}",
                config_path.display()
            );
            std::process::exit(1);
        }
    };

    // Lets daemon.toml (written by the wizard) override the systemd unit's
    // own Environment=PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file default —
    // must happen before the first `proton-drive` call below.
    if let Some(store) = &config.credentials_store {
        std::env::set_var("PROTON_DRIVE_CREDENTIALS_STORE", store);
    }

    let cache = match Cache::open(&Cache::default_db_path(), &Cache::default_root()) {
        Ok(cache) => cache,
        Err(err) => {
            log::error!("failed to open the pin cache: {err}");
            std::process::exit(1);
        }
    };

    control::start();

    let runner = RealCommandRunner;
    let notifier = notification::RealNotifier;
    let mut auth_notified = false;

    log::info!("reconciling pinned files under {}", cache.root().display());
    match sync::reconcile(&runner, &cache) {
        Ok(()) => auth_notified = false,
        Err(err) if err.is_authentication_error() => {
            report_authentication_failure(&mut auth_notified)
        }
        Err(err) => log::error!("initial reconcile failed: {err}"),
    }

    let (_watcher, events) = match watcher::watch(cache.root()) {
        Ok(handle) => handle,
        Err(err) => {
            log::error!("failed to watch {}: {err}", cache.root().display());
            std::process::exit(1);
        }
    };

    log::info!("watching {} for changes", cache.root().display());
    // Checked once immediately here (same shape as the reconcile() call
    // above), then every VERSION_CHECK_INTERVAL from inside the loop below.
    // Note this must NOT be `Instant::now()` followed by relying on the
    // loop's own elapsed() >= INTERVAL check to fire it "right away": a
    // freshly-started Instant has elapsed() ~0, which makes the *wait*
    // computed below ~INTERVAL (not ~0) — that would silently delay the
    // first check by a full day instead of running it at startup.
    let mut cli_update_notified: Option<String> = None;
    version_check::check(&runner, &notifier, &mut cli_update_notified);
    let mut last_version_check = Instant::now();
    // Same "must not be Instant::now() relied on for an immediate first
    // run" reasoning as `last_version_check` above — but the fs cache sweep
    // doesn't need one right at startup the way the version check does (a
    // freshly-started daemon's cache is whatever `bridge.rs` already wrote
    // on-demand, not urgently stale), so this one *does* start as
    // `Instant::now()`, deferring the first sweep by a full
    // `FS_CACHE_REFRESH_INTERVAL` instead.
    let mut last_fs_refresh = Instant::now();
    // Same reasoning as `last_fs_refresh` — no urgency for an immediate
    // first sweep at startup, so this also starts as `Instant::now()`.
    let mut last_cache_eviction = Instant::now();
    loop {
        let wait = VERSION_CHECK_INTERVAL
            .saturating_sub(last_version_check.elapsed())
            .min(FS_CACHE_REFRESH_INTERVAL.saturating_sub(last_fs_refresh.elapsed()))
            .min(CACHE_EVICTION_INTERVAL.saturating_sub(last_cache_eviction.elapsed()));
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
                        WatchEvent::Changed(path) => sync::upload_if_needed(&runner, &cache, path),
                        WatchEvent::Renamed { from, to } => {
                            sync::handle_rename(&runner, &cache, from, to)
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
            version_check::check(&runner, &notifier, &mut cli_update_notified);
            last_version_check = Instant::now();
        }

        if last_fs_refresh.elapsed() >= FS_CACHE_REFRESH_INTERVAL {
            fs_refresh::refresh_all(&runner, &cache);
            last_fs_refresh = Instant::now();
        }

        if last_cache_eviction.elapsed() >= CACHE_EVICTION_INTERVAL {
            cache_eviction::evict_stale(&cache, config.cache_retention());
            last_cache_eviction = Instant::now();
        }
    }
}

fn run_pin_client(action: &str, url: &str) {
    match control::run_client(action, url) {
        Ok(()) => println!("{action}ned {url}"),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
