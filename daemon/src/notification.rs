//! Best-effort desktop notification for authentication failures — a
//! `systemd --user` service has no other way to tell a human anything short
//! of the journal, which nobody watches proactively.

use std::process::Command;

use gettextrs::{
    bind_textdomain_codeset, bindtextdomain, gettext, setlocale, textdomain, LocaleCategory,
};

/// Separate translation domain from the KIO worker's `kio_protondrive` (see
/// `po/*/kio_protondrive_daemon.po`) — this binary ships in its own Debian
/// package (`kio-protondrive-sync-daemon`), installable independently of
/// the worker, and two packages both claiming the same
/// `/usr/share/locale/.../*.mo` path would conflict at the dpkg level.
const DOMAIN: &str = "kio_protondrive_daemon";
/// Matches `KDE_INSTALL_LOCALEDIR` on this distro (see
/// `debian/kio-protondrive-sync-daemon.install`) — hardcoded rather than
/// derived from the running binary's own path, since `daemon/` is plain
/// Cargo (no CMake/KDE ECM install-prefix plumbing reaches it).
const LOCALEDIR: &str = "/usr/share/locale";

/// Sets up gettext for this process — call once at startup, before any
/// notification function below. Best-effort: a locale/catalog that can't be
/// loaded just leaves the (English) `msgid`s as the effective text, same
/// "never fail the daemon over it" stance as the notify-send calls below.
pub fn init() {
    setlocale(LocaleCategory::LcAll, "");
    if let Err(err) = bindtextdomain(DOMAIN, LOCALEDIR) {
        log::debug!("could not bind the {DOMAIN} translation domain at {LOCALEDIR}: {err}");
        return;
    }
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    if let Err(err) = textdomain(DOMAIN) {
        log::debug!("could not activate the {DOMAIN} translation domain: {err}");
    }
}

/// Shells out to `notify-send` (part of `libnotify-bin`, a `Recommends` of
/// this package, not a hard `Depends` — a missing notifier shouldn't stop
/// the daemon from running, just means the user only finds out via
/// `journalctl --user -u kio-protondrive-sync-daemon`).
pub fn auth_required() {
    let result = Command::new("notify-send")
        .arg("--app-name=Proton Drive")
        .arg("--urgency=critical")
        .arg(gettext("Proton Drive: authentication required"))
        .arg(gettext(
            "Run \"proton-drive auth login\" in a terminal, then Proton Drive sync will resume automatically.",
        ))
        .status();
    if let Err(err) = result {
        log::debug!("could not send a desktop notification (notify-send missing?): {err}");
    }
}

/// See [`auth_required`] for the notify-send caveat. `message` is the
/// `proton-drive` CLI's own "A newer version is available: ..." sentence
/// (see [`crate::version_check`]), relayed as-is — untranslated, since it's
/// the external CLI's own English text, not ours to localize.
pub fn cli_update_available(message: &str) {
    let result = Command::new("notify-send")
        .arg("--app-name=Proton Drive")
        .arg("--urgency=normal")
        .arg(gettext("Proton Drive: CLI update available"))
        .arg(message)
        .status();
    if let Err(err) = result {
        log::debug!("could not send a desktop notification (notify-send missing?): {err}");
    }
}

/// Abstraction over "send the CLI-update-available notification", injectable
/// so [`crate::version_check`]'s tests never fire a real `notify-send` —
/// unlike `auth_required`, which no test calls into, this one *is* driven by
/// `version_check::check()`'s own unit tests, and a real notification
/// popping up on a developer's desktop every `cargo test` run is exactly the
/// kind of surprise this trait exists to prevent.
pub trait Notifier {
    fn cli_update_available(&self, message: &str);
}

#[derive(Debug, Default, Clone)]
pub struct RealNotifier;

impl Notifier for RealNotifier {
    fn cli_update_available(&self, message: &str) {
        cli_update_available(message);
    }
}
