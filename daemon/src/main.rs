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

use protondrive_core::cache::Cache;
use protondrive_core::cli::RealCommandRunner;

use kio_protondrive_daemon::config::Config;
use kio_protondrive_daemon::watcher::{self, WatchEvent};
use kio_protondrive_daemon::{control, notification, sync};

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
    for batch in events {
        for event in batch {
            let label = match &event {
                WatchEvent::Changed(path) => path.display().to_string(),
                WatchEvent::Renamed { from, to } => {
                    format!("{} -> {}", from.display(), to.display())
                }
            };
            let result = match &event {
                WatchEvent::Changed(path) => sync::upload_if_needed(&runner, &cache, path),
                WatchEvent::Renamed { from, to } => sync::handle_rename(&runner, &cache, from, to),
            };
            match result {
                Ok(()) => auth_notified = false,
                Err(err) if err.is_authentication_error() => {
                    report_authentication_failure(&mut auth_notified);
                    // The rest of this batch would fail identically — skip
                    // straight to the next debounced batch instead of
                    // burning a CLI call per remaining event.
                    break;
                }
                Err(err) => log::warn!("failed to sync {label}: {err} (will retry next cycle)"),
            }
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
