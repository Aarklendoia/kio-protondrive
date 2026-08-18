//! First-run setup wizard for kio-protondrive.
//!
//! Same architecture as the sibling `linux-hello-rust` project's GUI
//! (`linux_hello_config`): a dependency-free Rust launcher that spawns
//! Qt's own `qml6` runtime for the UI (see `qml/`) and runs a tiny
//! hand-rolled local HTTP control server the QML side calls into for real
//! logic — session checks, running `proton-drive auth login`, folder
//! validation, writing `daemon.toml`, setting up `pass`/GPG, and adding a
//! Dolphin favorite. No cxx-qt, no GUI/HTTP crate dependency; `core`/
//! `daemon` are path dependencies onto crates already in this workspace,
//! called in-process like any other Rust library.
//!
//! The control-server plumbing (port/token files, request parsing, JSON
//! escaping, the token check) lives in `protondrive_core::local_ctrl`,
//! shared with `daemon/src/control.rs`'s equivalent server — see that
//! module's doc comment for why a shared crate replaced what used to be
//! two independently-maintained copies.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use kio_protondrive_daemon::config::Config;
use protondrive_core::cli::{self, DriveError, RealCommandRunner};
use protondrive_core::local_ctrl::{
    self, constant_time_eq, extract_header, extract_query_param, generate_ctrl_token, json_escape,
    request_method, request_path, which, write_owner_only_file,
};

const APP_NAME: &str = "kio-protondrive-wizard";
const TOKEN_HEADER: &str = "x-kio-protondrive-wizard-token";

fn main() {
    let qml_path = find_qml_path();
    let uid = local_ctrl::current_uid();
    let runtime_dir = local_ctrl::runtime_dir(uid);

    let lock_path = runtime_dir.join(format!("{APP_NAME}.lock"));
    if let Ok(content) = std::fs::read_to_string(&lock_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if Path::new(&format!("/proc/{pid}")).exists() {
                eprintln!("Proton Drive setup wizard is already open (PID {pid}).");
                std::process::exit(0);
            }
        }
    }
    let _ = write_owner_only_file(&lock_path, &std::process::id().to_string());

    let ctrl_token = generate_ctrl_token();
    let ctrl_port = start_control_server(ctrl_token.clone());
    let ctrl_port_path = runtime_dir.join(format!("{APP_NAME}-ctrl.port"));
    let ctrl_token_path = runtime_dir.join(format!("{APP_NAME}-ctrl.token"));
    if let Err(e) = write_owner_only_file(&ctrl_port_path, &ctrl_port.to_string()) {
        eprintln!("Could not write the control port file: {e}");
    }
    if let Err(e) = write_owner_only_file(&ctrl_token_path, &ctrl_token) {
        eprintln!("Could not write the control token file: {e}");
    }

    // Queried via `qtpaths6` rather than a hardcoded multiarch triplet
    // (e.g. "x86_64-linux-gnu") — this package is `Architecture: any`, and
    // a hardcoded triplet silently fails to resolve QML/plugins on any
    // other one (e.g. arm64's "aarch64-linux-gnu"). Falls back to the
    // generic, non-arch-specific install paths if `qtpaths6` itself is
    // unavailable for some reason.
    let mut qml_import_dirs = vec![
        "/usr/share/qt6/qml".to_string(),
        "/usr/share/kio-protondrive-wizard/qml-modules".to_string(),
    ];
    if let Some(dir) = qt_query("QT_INSTALL_QML") {
        qml_import_dirs.insert(0, dir);
    }
    let qml_import_paths = qml_import_dirs.join(":");

    let mut qt_plugin_dirs = vec!["/usr/lib/qt6/plugins".to_string()];
    if let Some(dir) = qt_query("QT_INSTALL_PLUGINS") {
        qt_plugin_dirs.insert(0, dir);
    }
    let qt_plugin_paths = qt_plugin_dirs.join(":");

    // Trailing `-- <runtime_dir>` is how the QML side learns where to find
    // its own port/token files — Qt.environmentVariable isn't reliably
    // available to plain QML, but qml6 forwards anything after `--` into
    // Qt.application.arguments, which is. Passing the already-resolved
    // directory (rather than the bare UID and letting QML reconstruct
    // "/run/user/<uid>" itself) is deliberate: reconstructing it in QML
    // duplicates local_ctrl::runtime_dir's $XDG_RUNTIME_DIR-over-fallback
    // logic in a second language, and a prior version that did exactly
    // that silently broke on any system where $XDG_RUNTIME_DIR isn't
    // literally "/run/user/<uid>" (containers, some display managers) —
    // the whole wizard UI would hang on "Checking…" with no visible error.
    // Must stay the last argument (QML reads it by position).
    let mut cmd = Command::new("qml6");
    if let Some(qm_path) = resolve_locale().and_then(find_qml_translation_path) {
        cmd.arg("--translation").arg(qm_path);
    }
    cmd.arg(&qml_path)
        .arg("--")
        .arg(runtime_dir.to_string_lossy().into_owned())
        .env("QML_IMPORT_PATH", &qml_import_paths)
        .env("QML2_IMPORT_PATH", &qml_import_paths)
        .env("QT_PLUGIN_PATH", &qt_plugin_paths)
        .env("QT_QPA_PLATFORMTHEME", "kde")
        .env("QT_QUICK_CONTROLS_STYLE", "org.kde.desktop")
        .env("QT_APPLICATION_DISPLAY_NAME", "Proton Drive Setup")
        .env("QT_QPA_DESKTOPFILENAME", APP_NAME)
        .env("QML_XHR_ALLOW_FILE_READ", "1")
        .env("QT_QPA_PLATFORM", "xcb;wayland;offscreen");

    match cmd.spawn() {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(e) => {
            eprintln!("Could not launch qml6: {e}");
        }
    }

    let _ = std::fs::remove_file(&lock_path);
    let _ = std::fs::remove_file(&ctrl_port_path);
    let _ = std::fs::remove_file(&ctrl_token_path);
}

/// Asks Qt's own `qtpaths6 --query <var>` for a canonical install
/// directory (e.g. `QT_INSTALL_QML`, `QT_INSTALL_PLUGINS`) — the same tool
/// and query used to discover this manually while testing on this exact
/// distro, so it's the authoritative source rather than a guessed path.
fn qt_query(var: &str) -> Option<String> {
    let output = Command::new("qtpaths6")
        .args(["--query", var])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Installed `.qml` first, `$CARGO_MANIFEST_DIR/qml/main.qml` fallback for
/// `cargo run` during development.
fn find_qml_path() -> String {
    let candidates = [
        "/usr/share/kio-protondrive-wizard/qml/main.qml",
        "/usr/share/qt6/qml/KioProtondrive/Wizard/main.qml",
    ];
    for candidate in &candidates {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(&manifest_dir)
        .join("qml")
        .join("main.qml")
        .to_string_lossy()
        .to_string()
}

/// The 9 languages this project already covers for the KIO worker (KF6::I18n)
/// and the daemon (gettext, see `po/`) — kept in sync with those by hand,
/// since there's no shared source of truth across three different i18n
/// systems (KF6::I18n's `.po`, gettext's `.po`, and Qt Linguist's `.ts` here).
const SUPPORTED_LOCALES: &[&str] = &["ar", "de", "es", "fr", "hi", "ja", "pt_BR", "ru", "zh_CN"];

/// Resolves the user's locale to one of [`SUPPORTED_LOCALES`], following the
/// same `LC_ALL` > `LC_MESSAGES` > `LANG` precedence gettext/glibc use.
/// Tries the full value first (needed for `pt_BR`/`zh_CN`, which are
/// region-specific), then just the language part before `_` (so e.g.
/// `fr_FR.UTF-8` or `de_AT` still match `fr`/`de`) — `None` for an
/// unsupported or unset locale, which leaves `qsTr()` falling back to its
/// source (English) text, the correct default.
fn resolve_locale() -> Option<&'static str> {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .ok()?;
    let base = raw.split(['.', '@']).next().unwrap_or(&raw);
    SUPPORTED_LOCALES
        .iter()
        .copied()
        .find(|&l| l == base)
        .or_else(|| {
            let lang = base.split('_').next().unwrap_or(base);
            SUPPORTED_LOCALES.iter().copied().find(|&l| l == lang)
        })
}

/// The compiled `.qm` for `locale`, if this install (or dev tree) has one —
/// same install-then-dev-tree fallback pattern as [`find_qml_path`].
fn find_qml_translation_path(locale: &str) -> Option<PathBuf> {
    let installed = PathBuf::from(format!(
        "/usr/share/kio-protondrive-wizard/translations/kio_protondrive_wizard_{locale}.qm"
    ));
    if installed.is_file() {
        return Some(installed);
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let dev_path = PathBuf::from(&manifest_dir)
        .join("translations")
        .join(format!("kio_protondrive_wizard_{locale}.qm"));
    dev_path.is_file().then_some(dev_path)
}

fn start_control_server(token: String) -> u16 {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("unable to start the control server");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let token = token.clone();
            thread::spawn(move || handle_ctrl_connection(stream, &token));
        }
    });

    port
}

fn handle_ctrl_connection(mut stream: std::net::TcpStream, expected_token: &str) {
    use std::io::{Read, Write};
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).into_owned();

    let is_options = request_method(&req) == "OPTIONS";
    let token_ok = extract_header(&req, TOKEN_HEADER)
        .map(|t| constant_time_eq(t, expected_token))
        .unwrap_or(false);

    let (status, body): (&str, String) = if !is_options && !token_ok {
        ("403 Forbidden", String::new())
    } else if is_options {
        ("200 OK", String::new())
    } else {
        route(&req)
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Route dispatch — one `if` per route, matched on the request path (not a
/// raw substring match against the whole buffer, which a query value could
/// spoof). Each handler returns its own JSON body.
fn route(req: &str) -> (&'static str, String) {
    match request_path(req) {
        "/session-status" => ("200 OK", route_session_status()),
        "/auth-login" => ("200 OK", route_auth_login()),
        "/save-config" => ("200 OK", route_save_config(req)),
        "/credentials-status" => ("200 OK", route_credentials_status()),
        "/setup-pass" => ("200 OK", route_setup_pass(req)),
        "/add-favorite" => ("200 OK", route_add_favorite()),
        "/restart-daemon" => ("200 OK", route_restart_daemon()),
        _ => ("404 Not Found", "{}".to_string()),
    }
}

fn route_session_status() -> String {
    let runner = RealCommandRunner;
    let authenticated = !matches!(
        cli::stat_path(&runner, "/"),
        Err(DriveError::NotAuthenticated)
    );
    format!(r#"{{"authenticated":{authenticated}}}"#)
}

/// Blocks until `proton-drive auth login` exits — it opens a browser itself
/// and waits there for the user to finish (confirmed via
/// `proton-drive auth login -h`), so there's no URL/code to parse or
/// display; this connection's own thread is already isolated from the rest
/// of the server, so blocking here doesn't stall other requests.
fn route_auth_login() -> String {
    let status = Command::new("proton-drive")
        .args(["auth", "login"])
        .status();
    match status {
        Ok(status) if status.success() => route_session_status(),
        Ok(status) => format!(
            r#"{{"authenticated":false,"error":"proton-drive auth login exited with {}"}}"#,
            status.code().unwrap_or(-1)
        ),
        Err(e) => format!(
            r#"{{"authenticated":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// Persists the chosen credentials store to `daemon.toml` *and* applies it
/// to this wizard process's own environment — called right after
/// Credentials.qml, before Auth.qml runs `proton-drive auth login`, so that
/// login lands in the same store the daemon will actually read from
/// afterward. Without this, login would go to the CLI's own default
/// (the desktop keyring) regardless of what gets saved to daemon.toml,
/// leaving the daemon's chosen store empty — exactly the
/// "Dolphin works, the daemon says not logged in" bug this wizard exists to
/// prevent. `None`/empty means "unsafe_file" here even though that's saved
/// to disk as an absent key (see `Config::credentials_store`'s doc comment)
/// — the systemd unit's own `Environment=` only takes effect on the
/// *daemon's* next start, not in this already-running wizard process.
fn route_save_config(req: &str) -> String {
    let credentials_store = extract_query_param(req, "credentials_store").filter(|s| !s.is_empty());
    std::env::set_var(
        "PROTON_DRIVE_CREDENTIALS_STORE",
        credentials_store.as_deref().unwrap_or("unsafe_file"),
    );
    let config = Config { credentials_store };
    match config.save(&Config::default_path()) {
        Ok(()) => r#"{"ok":true}"#.to_string(),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// Reports what's already usable for the `pass` credentials store, so the
/// QML page can decide whether to offer "set it up" (both binaries present,
/// no key yet), "already ready" (a usable secret key exists), or "install
/// pass/gpg yourself first" (a binary's missing — this wizard never runs
/// `apt install`, see `route_setup_pass`).
fn route_credentials_status() -> String {
    let pass_installed = which("pass");
    let gpg_installed = which("gpg");
    let has_key = gpg_installed
        && Command::new("gpg")
            .args(["--list-secret-keys", "--with-colons"])
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false);
    format!(
        r#"{{"pass_installed":{pass_installed},"gpg_installed":{gpg_installed},"has_key":{has_key}}}"#
    )
}

/// Generates a GPG key (batch mode, no inline passphrase — gpg-agent's
/// pinentry prompts interactively for that, same as this project's own
/// git-commit signing) and runs `pass init` with it. Only ever reached once
/// `/credentials-status` has confirmed both `pass` and `gpg` are installed
/// — this never runs `apt install` anything itself (the user runs their own
/// `sudo`, project-wide rule). This whole step is optional/skippable in the
/// wizard UI, so a failure here is reported, never treated as fatal.
fn route_setup_pass(req: &str) -> String {
    if !which("pass") || !which("gpg") {
        return r#"{"ok":false,"error":"pass or gpg is not installed"}"#.to_string();
    }
    let Some(email) = extract_query_param(req, "email").filter(|e| e.contains('@')) else {
        return r#"{"ok":false,"error":"a valid email is required"}"#.to_string();
    };
    let name = std::env::var("USER").unwrap_or_else(|_| "Proton Drive".to_string());

    let key_id = match existing_secret_key_id() {
        Some(key_id) => key_id,
        None => {
            if let Err(e) = generate_gpg_key(&name, &email) {
                return format!(
                    r#"{{"ok":false,"error":"{}"}}"#,
                    json_escape(&e.to_string())
                );
            }
            let Some(key_id) = existing_secret_key_id() else {
                return r#"{"ok":false,"error":"gpg key generation did not produce a usable key"}"#
                    .to_string();
            };
            key_id
        }
    };

    match Command::new("pass").args(["init", &key_id]).status() {
        Ok(status) if status.success() => r#"{"ok":true}"#.to_string(),
        Ok(status) => format!(
            r#"{{"ok":false,"error":"pass init exited with {}"}}"#,
            status.code().unwrap_or(-1)
        ),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// The first secret key's fingerprint from `gpg --list-secret-keys
/// --with-colons`, if any — colon-format `fpr:` records follow the `sec:`
/// record for the key they belong to.
fn existing_secret_key_id() -> Option<String> {
    let output = Command::new("gpg")
        .args(["--list-secret-keys", "--with-colons"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    while let Some(line) = lines.next() {
        if line.starts_with("sec:") {
            if let Some(fpr_line) = lines.find(|l| l.starts_with("fpr:")) {
                return fpr_line.split(':').nth(9).map(str::to_string);
            }
        }
    }
    None
}

fn generate_gpg_key(name: &str, email: &str) -> std::io::Result<()> {
    let batch = format!(
        "%echo Generating a GPG key for kio-protondrive\n\
         Key-Type: eddsa\n\
         Key-Curve: ed25519\n\
         Subkey-Type: ecdh\n\
         Subkey-Curve: cv25519\n\
         Name-Real: {name}\n\
         Name-Email: {email}\n\
         Expire-Date: 2y\n\
         %commit\n\
         %echo done\n"
    );
    let batch_path = std::env::temp_dir().join(format!(
        "kio-protondrive-wizard-gpg-batch-{}.tmp",
        std::process::id()
    ));
    std::fs::write(&batch_path, batch)?;
    let status = Command::new("gpg")
        .args(["--batch", "--gen-key"])
        .arg(&batch_path)
        .status();
    let _ = std::fs::remove_file(&batch_path);
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(std::io::Error::other(format!(
            "gpg --gen-key exited with {}",
            status.code().unwrap_or(-1)
        ))),
        Err(e) => Err(e),
    }
}

/// Best-effort append of a `protondrive:/` bookmark into Dolphin's Places
/// panel (`~/.local/share/user-places.xbel`). Points at the root rather than
/// `protondrive:/my-files` so every virtual section (My files, Photos,
/// Shared, ...) is one click away, matching Proton Drive's own web UI
/// sidebar instead of hiding everything but My files behind a single
/// shortcut. Plain text insertion before `</xbel>` rather than pulling in an
/// XML crate: this step is optional and best-effort (same tolerance as
/// `daemon/src/notification.rs` for a missing `notify-send`), so a
/// malformed/unexpected file is just skipped rather than risked being
/// corrupted by a naive parser.
fn route_add_favorite() -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return r#"{"ok":false,"error":"HOME not set"}"#.to_string();
    };
    let xbel_path = PathBuf::from(home).join(".local/share/user-places.xbel");
    let existing = std::fs::read_to_string(&xbel_path).unwrap_or_default();

    if existing.contains("protondrive:/") {
        return r#"{"ok":true}"#.to_string();
    }
    let Some(insert_at) = existing.rfind("</xbel>") else {
        return r#"{"ok":false,"error":"user-places.xbel not found or not well-formed"}"#
            .to_string();
    };

    // The icon must be a `bookmark:icon` element (freedesktop desktop-bookmark
    // namespace, as declared on the root `<xbel>` element) under an
    // `owner="http://freedesktop.org"` metadata block, matching exactly how
    // Dolphin itself writes its own built-in places (Home, Documents, ...).
    // A plain `<icon>` under the KDE metadata block is silently ignored by
    // KFilePlacesModel's parser, leaving the entry with a "?" icon.
    let bookmark = "  <bookmark href=\"protondrive:/\">\n    \
                     <title>Proton Drive</title>\n    \
                     <info>\n      <metadata owner=\"http://freedesktop.org\">\n        \
                     <bookmark:icon name=\"folder-cloud\"/>\n      </metadata>\n    </info>\n  \
                     </bookmark>\n";
    let mut updated = existing.clone();
    updated.insert_str(insert_at, bookmark);

    match std::fs::write(&xbel_path, updated) {
        Ok(()) => r#"{"ok":true}"#.to_string(),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

fn route_restart_daemon() -> String {
    let status = Command::new("systemctl")
        .args(["--user", "restart", "kio-protondrive-sync-daemon.service"])
        .status();
    match status {
        Ok(status) if status.success() => r#"{"ok":true}"#.to_string(),
        Ok(status) => format!(
            r#"{{"ok":false,"error":"systemctl exited with {}"}}"#,
            status.code().unwrap_or(-1)
        ),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_add_favorite_inserts_a_bookmark_before_the_closing_tag() {
        let dir = std::env::temp_dir().join(format!(
            "kio-protondrive-wizard-xbel-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Isolate HOME so this test never touches the real machine's
        // Dolphin bookmarks.
        std::env::set_var("HOME", &dir);
        std::fs::create_dir_all(dir.join(".local/share")).unwrap();
        std::fs::write(
            dir.join(".local/share/user-places.xbel"),
            "<xbel>\n</xbel>\n",
        )
        .unwrap();

        let result = route_add_favorite();
        assert!(result.contains("\"ok\":true"));
        let updated = std::fs::read_to_string(dir.join(".local/share/user-places.xbel")).unwrap();
        assert!(updated.contains("href=\"protondrive:/\""));

        // Calling it again should be a no-op, not a duplicate entry.
        let result2 = route_add_favorite();
        assert!(result2.contains("\"ok\":true"));
        let updated2 = std::fs::read_to_string(dir.join(".local/share/user-places.xbel")).unwrap();
        assert_eq!(
            updated2.matches("href=\"protondrive:/\"").count(),
            1,
            "the bookmark should only be inserted once"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // Both scenarios live in one test (rather than two `#[test]`s) because
    // they mutate the process-wide PROTON_DRIVE_CREDENTIALS_STORE env var —
    // cargo runs tests in parallel threads by default, so two separate
    // tests race on it (confirmed live: intermittent failures asserting the
    // other test's value).
    #[test]
    fn save_config_applies_the_credentials_store_to_the_live_env() {
        let dir = std::env::temp_dir().join(format!(
            "kio-protondrive-wizard-save-config-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        let result = route_save_config("GET /save-config HTTP/1.1");
        assert!(result.contains("\"ok\":true"));
        assert_eq!(
            std::env::var("PROTON_DRIVE_CREDENTIALS_STORE").as_deref(),
            Ok("unsafe_file"),
            "an unspecified store must still resolve to the daemon's actual \
             default, not the CLI's own (the desktop keyring) — otherwise \
             the login this triggers next lands in the wrong place"
        );
        let saved = std::fs::read_to_string(dir.join("kio-protondrive/daemon.toml")).unwrap();
        assert!(
            !saved.contains("credentials_store"),
            "unsafe_file is the implicit default and shouldn't be written out"
        );

        let result = route_save_config("GET /save-config?credentials_store=pass HTTP/1.1");
        assert!(result.contains("\"ok\":true"));
        assert_eq!(
            std::env::var("PROTON_DRIVE_CREDENTIALS_STORE").as_deref(),
            Ok("pass")
        );
        let saved = std::fs::read_to_string(dir.join("kio-protondrive/daemon.toml")).unwrap();
        assert!(saved.contains("credentials_store = \"pass\""));

        std::fs::remove_dir_all(&dir).ok();
    }

    // One test, sequential scenarios — same reasoning as
    // save_config_applies_the_credentials_store_to_the_live_env above:
    // LC_ALL/LC_MESSAGES/LANG are process-wide, so parallel `#[test]`s
    // mutating them would race.
    #[test]
    fn resolve_locale_matches_supported_languages_with_fallback() {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            std::env::remove_var(var);
        }

        std::env::set_var("LANG", "fr_FR.UTF-8");
        assert_eq!(resolve_locale(), Some("fr"), "language part of LANG");

        std::env::set_var("LANG", "pt_BR.UTF-8");
        assert_eq!(
            resolve_locale(),
            Some("pt_BR"),
            "full region-specific value must be tried before the bare language part"
        );

        std::env::set_var("LANG", "de_AT@euro");
        assert_eq!(resolve_locale(), Some("de"), "modifier suffix stripped");

        std::env::set_var("LC_MESSAGES", "ja_JP.UTF-8");
        assert_eq!(
            resolve_locale(),
            Some("ja"),
            "LC_MESSAGES takes priority over LANG"
        );

        std::env::set_var("LC_ALL", "ru_RU.UTF-8");
        assert_eq!(
            resolve_locale(),
            Some("ru"),
            "LC_ALL takes priority over LC_MESSAGES and LANG"
        );

        std::env::remove_var("LC_ALL");
        std::env::remove_var("LC_MESSAGES");
        std::env::set_var("LANG", "pt_PT.UTF-8");
        assert_eq!(
            resolve_locale(),
            None,
            "a region variant this project doesn't cover (only pt_BR) must not \
             fall back to some other pt_* match"
        );

        std::env::set_var("LANG", "C");
        assert_eq!(resolve_locale(), None);

        std::env::remove_var("LANG");
        assert_eq!(resolve_locale(), None, "no locale env vars set at all");
    }
}
