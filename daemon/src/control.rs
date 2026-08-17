//! Local control server for `kio-protondrive-daemon pin <url>` / `unpin
//! <url>` — built on `protondrive_core::local_ctrl`'s shared hand-rolled
//! local-HTTP plumbing (port/token files under `$XDG_RUNTIME_DIR`, manual
//! request parsing, a per-run token gating every route), same as
//! `wizard/src/main.rs`'s own control server. See that module's doc
//! comments for the full security reasoning behind each piece.
//!
//! Unlike the wizard (whose QML frontend can't read env vars and needs its
//! own UID passed as a trailing process argument to find its files), the
//! client side here (`run_client`) is just another invocation of this same
//! binary, running as a normal process with full env access — no
//! UID-passing workaround needed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

use protondrive_core::cache::Cache;
use protondrive_core::cli::RealCommandRunner;
use protondrive_core::local_ctrl::{
    self, constant_time_eq, extract_header, extract_query_param, generate_ctrl_token, json_escape,
    percent_encode, request_path, write_owner_only_file,
};

const APP_NAME: &str = "kio-protondrive-daemon";
const TOKEN_HEADER: &str = "x-kio-protondrive-daemon-token";

fn ctrl_port_path() -> PathBuf {
    local_ctrl::runtime_dir(local_ctrl::current_uid()).join(format!("{APP_NAME}-ctrl.port"))
}

fn ctrl_token_path() -> PathBuf {
    local_ctrl::runtime_dir(local_ctrl::current_uid()).join(format!("{APP_NAME}-ctrl.token"))
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

    let token_ok = extract_header(&req, TOKEN_HEADER)
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

/// `force=1` in the query string bypasses `Cache::pin`/`Cache::unpin`'s
/// unsynced-local-edits guard — opt-in, since the default behavior is to
/// refuse rather than silently discard an edit the daemon hasn't uploaded
/// yet (see `core::cache::Cache::pin`/`unpin`'s own docs).
fn force_param(req: &str) -> bool {
    matches!(
        extract_query_param(req, "force").as_deref(),
        Some("1") | Some("true")
    )
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
    match cache.pin(&runner, &path, force_param(req)) {
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
    match cache.unpin(&path, force_param(req)) {
        Ok(()) => r#"{"ok":true}"#.to_string(),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// `kio-protondrive-daemon pin <url>` / `unpin <url>` client mode — what
/// the Dolphin ServiceMenu action (see `daemon/kio-protondrive-pin.desktop`)
/// actually invokes. `url` is the raw `protondrive:/...` URL KIO hands the
/// action via `%u`; strips the scheme down to the bare Drive path
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
    fn force_param_reads_1_or_true() {
        assert!(force_param("GET /unpin?path=%2Fa&force=1 HTTP/1.1\r\n"));
        assert!(force_param("GET /unpin?path=%2Fa&force=true HTTP/1.1\r\n"));
        assert!(!force_param("GET /unpin?path=%2Fa HTTP/1.1\r\n"));
        assert!(!force_param("GET /unpin?path=%2Fa&force=0 HTTP/1.1\r\n"));
    }
}
