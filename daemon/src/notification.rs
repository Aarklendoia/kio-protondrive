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

/// The last path segment, for display in a notification — a bare filename
/// reads better than the full Drive path, and (unlike the path) never needs
/// translation, so it's kept out of any gettext msgid.
fn display_name(remote_path: &str) -> &str {
    remote_path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(remote_path)
}

/// Fires a persistent "downloading…" notification for a pin's download —
/// mirrors what Dolphin shows for a copy job. `proton-drive filesystem
/// download` is a single blocking CLI call with no incremental progress to
/// report, so this is deliberately indeterminate (no percentage), just
/// enough to answer "is anything happening?" — previously the answer was a
/// silent wait with no feedback either way, success or failure.
///
/// Returns the notification's id (`notify-send --print-id`'s stdout) so
/// [`pin_finished`] can replace it in place rather than leaving a stale
/// "downloading…" notification sitting next to a new one; `None` (id
/// unavailable, e.g. `notify-send` missing) just means `pin_finished` sends
/// a fresh notification instead of replacing.
pub fn pin_started(remote_path: &str) -> Option<String> {
    let output = Command::new("notify-send")
        .arg("--app-name=Proton Drive")
        .arg("--urgency=low")
        .arg("--expire-time=0")
        .arg("--print-id")
        .arg(display_name(remote_path))
        .arg(gettext("Downloading to keep available offline…"))
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!id.is_empty()).then_some(id)
        }
        Ok(_) => None,
        Err(err) => {
            log::debug!("could not send a desktop notification (notify-send missing?): {err}");
            None
        }
    }
}

/// Replaces (or, absent an `id`, sends fresh) the [`pin_started`]
/// notification with the outcome — `error` being `Some` also fixes what
/// used to be a silent failure (the pin CLI's stderr is never seen, since
/// the ServiceMenu that invokes it runs with no visible terminal): a failed
/// pin was previously indistinguishable from "still downloading" and from
/// "succeeded", all three looking exactly like nothing happened.
pub fn pin_finished(id: Option<&str>, remote_path: &str, error: Option<&str>) {
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name=Proton Drive");
    if let Some(id) = id {
        cmd.arg(format!("--replace-id={id}"));
    }
    let body = match error {
        None => {
            cmd.arg("--urgency=low").arg("--expire-time=5000");
            gettext("Now available offline.")
        }
        Some(error) => {
            cmd.arg("--urgency=critical").arg("--expire-time=0");
            format!("{} {error}", gettext("Could not keep available offline:"))
        }
    };
    cmd.arg(display_name(remote_path)).arg(body);
    if let Err(err) = cmd.status() {
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
