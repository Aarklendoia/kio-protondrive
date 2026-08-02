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

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use kio_protondrive_daemon::config::Config;
use protondrive_core::cli::{self, DriveError, RealCommandRunner};

const APP_NAME: &str = "kio-protondrive-wizard";

fn main() {
    let qml_path = find_qml_path();
    let uid = get_current_uid();

    let lock_path = format!("{}/{APP_NAME}.lock", runtime_dir(uid));
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
    if let Err(e) = write_owner_only_file(&ctrl_port_path(uid), &ctrl_port.to_string()) {
        eprintln!("Could not write the control port file: {e}");
    }
    if let Err(e) = write_owner_only_file(&ctrl_token_path(uid), &ctrl_token) {
        eprintln!("Could not write the control token file: {e}");
    }

    let qml_import_paths = [
        "/usr/lib/x86_64-linux-gnu/qt6/qml",
        "/usr/share/qt6/qml",
        "/usr/share/kio-protondrive-wizard/qml-modules",
    ]
    .join(":");
    let qt_plugin_paths = [
        "/usr/lib/x86_64-linux-gnu/qt6/plugins",
        "/usr/lib/qt6/plugins",
    ]
    .join(":");

    // Trailing `-- <uid>` is how the QML side learns its own UID (to find
    // its own namespaced port/token files under $XDG_RUNTIME_DIR) —
    // Qt.environmentVariable isn't reliably available to plain QML, but
    // qml6 forwards anything after `--` into Qt.application.arguments,
    // which is. Must stay the last argument (QML reads it by position).
    let mut cmd = Command::new("qml6");
    cmd.arg(&qml_path)
        .arg("--")
        .arg(uid.to_string())
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
    let _ = std::fs::remove_file(ctrl_port_path(uid));
    let _ = std::fs::remove_file(ctrl_token_path(uid));
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

fn get_current_uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1000)
}

/// Base directory for the control server's port/token/lock files —
/// `$XDG_RUNTIME_DIR` (falls back to `/run/user/<uid>`), never `/tmp`: a
/// different-UID attacker could pre-plant a symlink at a predictable
/// `/tmp/...` path pointing somewhere we can write, which the owner-only
/// file writer below would then follow. `$XDG_RUNTIME_DIR` is per-user,
/// mode 0700, owned solely by this UID.
fn runtime_dir(uid: u32) -> String {
    std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{uid}"))
}

fn ctrl_port_path(uid: u32) -> String {
    format!("{}/{APP_NAME}-ctrl.port", runtime_dir(uid))
}

fn ctrl_token_path(uid: u32) -> String {
    format!("{}/{APP_NAME}-ctrl.token", runtime_dir(uid))
}

/// 64 lowercase hex chars from `/dev/urandom` — used to authenticate
/// requests to the local control server (see `handle_ctrl_connection`).
/// `read_exact`, not `fs::read`: the latter would block forever on a
/// character device that never returns EOF.
fn generate_ctrl_token() -> String {
    use std::fs::File;
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("unable to read /dev/urandom for the control server token");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `contents` to `path` readable/writable only by the current user
/// (mode 0600). Removes any pre-existing file at `path` first (a stale
/// leftover from a crashed prior run) and uses `create_new` so a
/// same-UID TOCTOU race can't slip a different file in between the
/// remove and the write.
fn write_owner_only_file(path: &str, contents: &str) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(path);
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

/// Escapes a string for embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Decodes `%XX` percent-escapes and `+` (space) in a URL query value.
/// Malformed escapes are passed through literally rather than erroring —
/// this is a local, trusted, loopback-only control server, not a public
/// HTTP endpoint that needs to defend against adversarial encoding.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extracts `name`'s value from the query string of an HTTP request's first
/// line (e.g. `GET /route?path=%2Fhome HTTP/1.1`). Every route on this
/// server takes its parameters this way (GET or POST alike) rather than a
/// parsed body — there's no request here that needs more than a handful of
/// short string values, so a body parser (and the Content-Length handling
/// it'd need) isn't worth adding.
fn extract_query_param(req: &str, name: &str) -> Option<String> {
    let request_line = req.lines().next()?;
    let path_and_query = request_line.split_whitespace().nth(1)?;
    let query = path_and_query.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decode(value))
    })
}

fn extract_token_header(req: &str) -> Option<&str> {
    req.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .starts_with("x-kio-protondrive-wizard-token:")
            .then(|| line["x-kio-protondrive-wizard-token:".len()..].trim())
    })
}

fn request_method(req: &str) -> &str {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("")
}

fn request_path(req: &str) -> &str {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
}

/// Constant-time string comparison — used for the token check so a
/// co-resident local process can't use response-timing differences to
/// recover the token faster than brute force. The length check still
/// returns early, but the token's length isn't secret (always exactly 64
/// hex chars, see `generate_ctrl_token`).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn start_control_server(token: String) -> u16 {
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

fn handle_ctrl_connection(mut stream: TcpStream, expected_token: &str) {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).into_owned();

    let is_options = request_method(&req) == "OPTIONS";
    let token_ok = extract_token_header(&req)
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

fn route_save_config(req: &str) -> String {
    let credentials_store = extract_query_param(req, "credentials_store").filter(|s| !s.is_empty());
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

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
            match existing_secret_key_id() {
                Some(key_id) => key_id,
                None => {
                    return r#"{"ok":false,"error":"gpg key generation did not produce a usable key"}"#
                        .to_string();
                }
            }
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

/// Best-effort append of a `protondrive:/my-files` bookmark into Dolphin's
/// Places panel (`~/.local/share/user-places.xbel`). Plain text insertion
/// before `</xbel>` rather than pulling in an XML crate: this step is
/// optional and best-effort (same tolerance as `daemon/src/notification.rs`
/// for a missing `notify-send`), so a malformed/unexpected file is just
/// skipped rather than risked being corrupted by a naive parser.
fn route_add_favorite() -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return r#"{"ok":false,"error":"HOME not set"}"#.to_string();
    };
    let xbel_path = PathBuf::from(home).join(".local/share/user-places.xbel");
    let existing = std::fs::read_to_string(&xbel_path).unwrap_or_default();

    if existing.contains("protondrive:/my-files") {
        return r#"{"ok":true}"#.to_string();
    }
    let Some(insert_at) = existing.rfind("</xbel>") else {
        return r#"{"ok":false,"error":"user-places.xbel not found or not well-formed"}"#
            .to_string();
    };

    let bookmark = "  <bookmark href=\"protondrive:/my-files\">\n    \
                     <title>Proton Drive</title>\n    \
                     <info>\n      <metadata owner=\"http://www.kde.org\">\n        \
                     <icon name=\"folder-cloud\"/>\n      </metadata>\n    </info>\n  \
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
    fn generate_ctrl_token_is_64_lowercase_hex_chars_and_varies() {
        let a = generate_ctrl_token();
        let b = generate_ctrl_token();
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn write_owner_only_file_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "kio-protondrive-wizard-test-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_str = path.to_str().unwrap();
        write_owner_only_file(path_str, "secret").unwrap();
        let mode = std::fs::metadata(path_str).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(path_str).unwrap(), "secret");
        let _ = std::fs::remove_file(path_str);
    }

    #[test]
    fn json_escape_handles_quotes_backslashes_and_control_chars() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_decode("no-escapes"), "no-escapes");
    }

    #[test]
    fn extract_query_param_reads_a_value_from_the_request_line() {
        let req = "GET /validate-local-folder?path=%2Fhome%2Fuser HTTP/1.1\r\nHost: x\r\n";
        assert_eq!(
            extract_query_param(req, "path"),
            Some("/home/user".to_string())
        );
        assert_eq!(extract_query_param(req, "missing"), None);
    }

    #[test]
    fn extract_token_header_is_case_insensitive() {
        let req = "GET / HTTP/1.1\r\nX-Kio-Protondrive-Wizard-Token: abc123\r\n";
        assert_eq!(extract_token_header(req), Some("abc123"));
    }

    #[test]
    fn request_path_strips_the_query_string() {
        assert_eq!(request_path("GET /route?a=1&b=2 HTTP/1.1\r\n"), "/route");
    }

    #[test]
    fn constant_time_eq_matches_regular_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }

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
        assert!(updated.contains("protondrive:/my-files"));

        // Calling it again should be a no-op, not a duplicate entry.
        let result2 = route_add_favorite();
        assert!(result2.contains("\"ok\":true"));
        let updated2 = std::fs::read_to_string(dir.join(".local/share/user-places.xbel")).unwrap();
        assert_eq!(
            updated2.matches("protondrive:/my-files").count(),
            1,
            "the bookmark should only be inserted once"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
