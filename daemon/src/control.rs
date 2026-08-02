//! Local control server for `kio-protondrive-daemon pin <url>` / `unpin
//! <url>` — same hand-rolled local-HTTP pattern as `wizard/src/main.rs`'s
//! control server (port/token files under `$XDG_RUNTIME_DIR`, manual
//! request parsing, manual JSON escaping, a per-run token gating every
//! route). See that file's comments for the full security reasoning
//! behind each piece; not repeated here. Different binary, so this is a
//! small, deliberate duplication rather than a shared crate for ~150
//! lines.
//!
//! Unlike the wizard (whose QML frontend can't read env vars and needs
//! its own UID passed as a trailing process argument to find its files),
//! the client side here (`run_client`) is just another invocation of this
//! same binary, running as a normal process with full env access — no
//! UID-passing workaround needed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

use protondrive_core::cache::Cache;
use protondrive_core::cli::RealCommandRunner;

const APP_NAME: &str = "kio-protondrive-daemon";

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn ctrl_port_path() -> PathBuf {
    runtime_dir().join(format!("{APP_NAME}-ctrl.port"))
}

fn ctrl_token_path() -> PathBuf {
    runtime_dir().join(format!("{APP_NAME}-ctrl.token"))
}

fn generate_ctrl_token() -> String {
    use std::fs::File;
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("unable to read /dev/urandom for the control server token");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_owner_only_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
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

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

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
            .starts_with("x-kio-protondrive-daemon-token:")
            .then(|| line["x-kio-protondrive-daemon-token:".len()..].trim())
    })
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

/// Constant-time comparison — see `wizard/src/main.rs`'s identical helper
/// for why (avoids response-timing side channels on the token check).
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

/// Starts the control server on a background thread; writes its port/token
/// files. Best-effort — a failure here (e.g. can't bind a loopback socket)
/// is logged and otherwise non-fatal, same tolerance as
/// `notification.rs`'s `notify-send` shell-out: pinning just won't work
/// until the next restart, but syncing already-pinned files still does.
pub fn start() {
    let token = generate_ctrl_token();
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) => {
            log::warn!("could not start the pin control server: {err}");
            return;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    if let Err(err) = write_owner_only_file(&ctrl_port_path(), &port.to_string()) {
        log::warn!("could not write the control port file: {err}");
    }
    if let Err(err) = write_owner_only_file(&ctrl_token_path(), &token) {
        log::warn!("could not write the control token file: {err}");
    }

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let token = token.clone();
            thread::spawn(move || handle_connection(stream, &token));
        }
    });
}

fn handle_connection(mut stream: TcpStream, expected_token: &str) {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).into_owned();

    let token_ok = extract_token_header(&req)
        .map(|t| constant_time_eq(t, expected_token))
        .unwrap_or(false);

    let (status, body): (&str, String) = if !token_ok {
        ("403 Forbidden", String::new())
    } else {
        match request_path(&req) {
            "/pin" => ("200 OK", route_pin(&req)),
            "/unpin" => ("200 OK", route_unpin(&req)),
            _ => ("404 Not Found", "{}".to_string()),
        }
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn route_pin(req: &str) -> String {
    let Some(path) = extract_query_param(req, "path") else {
        return r#"{"ok":false,"error":"missing path"}"#.to_string();
    };
    let cache = match Cache::open(&Cache::default_db_path(), &Cache::default_root()) {
        Ok(c) => c,
        Err(e) => {
            return format!(
                r#"{{"ok":false,"error":"{}"}}"#,
                json_escape(&e.to_string())
            )
        }
    };
    let runner = RealCommandRunner;
    match cache.pin(&runner, &path) {
        Ok(local) => format!(
            r#"{{"ok":true,"local_path":"{}"}}"#,
            json_escape(&local.to_string_lossy())
        ),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

fn route_unpin(req: &str) -> String {
    let Some(path) = extract_query_param(req, "path") else {
        return r#"{"ok":false,"error":"missing path"}"#.to_string();
    };
    let cache = match Cache::open(&Cache::default_db_path(), &Cache::default_root()) {
        Ok(c) => c,
        Err(e) => {
            return format!(
                r#"{{"ok":false,"error":"{}"}}"#,
                json_escape(&e.to_string())
            )
        }
    };
    match cache.unpin(&path) {
        Ok(()) => r#"{"ok":true}"#.to_string(),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// `kio-protondrive-daemon pin <url>` / `unpin <url>` client mode — what
/// the Dolphin ServiceMenu action (see `debian/kio-protondrive.protondrive-pin.desktop`)
/// actually invokes. `url` is the raw `protondrive:/...` URL KIO hands the
/// action via `%U`; strips the scheme down to the bare Drive path
/// `cache::Cache`/`core::cli` expect everywhere else (e.g.
/// "protondrive:/my-files/a.pdf" -> "/my-files/a.pdf").
pub fn run_client(action: &str, url: &str) -> Result<(), String> {
    let remote_path = url
        .strip_prefix("protondrive:")
        .unwrap_or(url)
        .trim_start_matches('/');
    let remote_path = format!("/{remote_path}");

    let port = std::fs::read_to_string(ctrl_port_path())
        .map_err(|e| format!("is the daemon running? ({e})"))?;
    let token = std::fs::read_to_string(ctrl_token_path()).map_err(|e| e.to_string())?;

    let route = match action {
        "pin" => "/pin",
        "unpin" => "/unpin",
        other => return Err(format!("unknown action {other:?}")),
    };
    let request = format!(
        "GET {route}?path={} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Kio-Protondrive-Daemon-Token: {}\r\nConnection: close\r\n\r\n",
        percent_encode(&remote_path),
        token.trim(),
    );

    let mut stream =
        TcpStream::connect(format!("127.0.0.1:{}", port.trim())).map_err(|e| e.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| e.to_string())?;

    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("");
    if body.contains("\"ok\":true") {
        Ok(())
    } else {
        Err(body.to_string())
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
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
    }

    #[test]
    fn percent_encode_round_trips_through_percent_decode() {
        let path = "/my-files/Reports/q3 report (final).pdf";
        assert_eq!(percent_decode(&percent_encode(path)), path);
    }

    #[test]
    fn extract_query_param_reads_a_value() {
        let req = "GET /pin?path=%2Fmy-files%2Fa.txt HTTP/1.1\r\n";
        assert_eq!(
            extract_query_param(req, "path"),
            Some("/my-files/a.txt".to_string())
        );
    }

    #[test]
    fn extract_token_header_is_case_insensitive() {
        let req = "GET / HTTP/1.1\r\nX-Kio-Protondrive-Daemon-Token: abc\r\n";
        assert_eq!(extract_token_header(req), Some("abc"));
    }

    #[test]
    fn request_path_strips_the_query_string() {
        assert_eq!(request_path("GET /pin?path=x HTTP/1.1\r\n"), "/pin");
    }

    #[test]
    fn constant_time_eq_matches_regular_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
    }
}
