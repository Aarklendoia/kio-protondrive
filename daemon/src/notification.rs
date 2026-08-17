//! Best-effort desktop notification for authentication failures — a
//! `systemd --user` service has no other way to tell a human anything short
//! of the journal, which nobody watches proactively.

use std::process::Command;

/// Shells out to `notify-send` (part of `libnotify-bin`, a `Recommends` of
/// this package, not a hard `Depends` — a missing notifier shouldn't stop
/// the daemon from running, just means the user only finds out via
/// `journalctl --user -u kio-protondrive-sync-daemon`).
pub fn auth_required() {
    let result = Command::new("notify-send")
        .args([
            "--app-name=Proton Drive",
            "--urgency=critical",
            "Proton Drive: authentication required",
            "Run \"proton-drive auth login\" in a terminal, then Proton Drive sync will resume automatically.",
        ])
        .status();
    if let Err(err) = result {
        log::debug!("could not send a desktop notification (notify-send missing?): {err}");
    }
}

/// See [`auth_required`] for the notify-send caveat. `message` is the
/// `proton-drive` CLI's own "A newer version is available: ..." sentence
/// (see [`crate::version_check`]), relayed as-is.
pub fn cli_update_available(message: &str) {
    let result = Command::new("notify-send")
        .args([
            "--app-name=Proton Drive",
            "--urgency=normal",
            "Proton Drive: CLI update available",
            message,
        ])
        .status();
    if let Err(err) = result {
        log::debug!("could not send a desktop notification (notify-send missing?): {err}");
    }
}
