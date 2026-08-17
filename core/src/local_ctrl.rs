//! Shared plumbing for the two hand-rolled local HTTP control servers used
//! by `daemon/` and `wizard/` (port/token files under `$XDG_RUNTIME_DIR`,
//! manual request parsing, token-gated routing).
//!
//! Extracted after the two independent copies of this ~150 lines had
//! already drifted apart in a security-relevant way: the daemon's copy had
//! regressed to a world-writable `/tmp` fallback the wizard's never had
//! (see [`runtime_dir`]'s doc for why that matters). One copy can't drift
//! from a second that no longer exists.
//!
//! Loopback-only (`127.0.0.1`), single-user, trusted-local-process threat
//! model throughout — the token check defends against another local user
//! guessing/observing the port, not against a network attacker. This is not
//! a public-facing HTTP server hardening exercise.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Command;

/// Base directory for control-server port/token/lock files —
/// `$XDG_RUNTIME_DIR`, falling back to `/run/user/<uid>`, **never** `/tmp`:
/// a different-UID attacker could pre-plant a symlink at a predictable
/// `/tmp/...` path pointing somewhere writable, which [`write_owner_only_file`]
/// would then follow. `$XDG_RUNTIME_DIR` (or its fallback) is per-user, mode
/// 0700, so no other UID can even traverse into it.
pub fn runtime_dir(uid: u32) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")))
}

/// The current process's UID, via `id -u` — used only to compute
/// [`runtime_dir`]'s fallback path, so a shell-out here (rather than a
/// `libc` dependency for one `getuid()` call) is cheap enough: called once
/// at startup, not per request.
pub fn current_uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1000)
}

/// 64 lowercase hex chars from `/dev/urandom` — used to authenticate
/// requests to a control server. `read_exact`, not `fs::read`: the latter
/// would block forever on a character device that never returns EOF.
pub fn generate_ctrl_token() -> String {
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("unable to read /dev/urandom for the control server token");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `contents` to `path` readable/writable only by the current user
/// (mode 0600). Removes any pre-existing file at `path` first (a stale
/// leftover from a crashed prior run) and uses `create_new` so a same-UID
/// TOCTOU race can't slip a different file in between the remove and the
/// write.
pub fn write_owner_only_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

/// Escapes a string for embedding in a JSON string literal.
pub fn json_escape(s: &str) -> String {
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
pub fn percent_decode(s: &str) -> String {
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

/// Percent-encodes a string for use as a URL query value — the inverse of
/// [`percent_decode`], used by a client request rather than the server
/// handling one.
pub fn percent_encode(s: &str) -> String {
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

/// Extracts `name`'s value from the query string of an HTTP request's first
/// line (e.g. `GET /route?path=%2Fhome HTTP/1.1`). Every route on either
/// control server takes its parameters this way (GET or POST alike) rather
/// than a parsed body.
pub fn extract_query_param(req: &str, name: &str) -> Option<String> {
    let request_line = req.lines().next()?;
    let path_and_query = request_line.split_whitespace().nth(1)?;
    let query = path_and_query.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decode(value))
    })
}

/// Extracts the value of `header_name`'s header line, case-insensitively —
/// each control server uses its own header name (e.g.
/// `x-kio-protondrive-daemon-token`) so a co-resident wizard/daemon
/// instance can't accidentally authenticate against the other.
pub fn extract_header<'a>(req: &'a str, header_name: &str) -> Option<&'a str> {
    let prefix = format!("{header_name}:");
    req.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .starts_with(&prefix)
            .then(|| line[prefix.len()..].trim())
    })
}

pub fn request_method(req: &str) -> &str {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("")
}

pub fn request_path(req: &str) -> &str {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
}

/// Constant-time string comparison — used for token checks so a co-resident
/// local process can't use response-timing differences to recover the
/// token faster than brute force. The length check still returns early,
/// but the token's length isn't secret (always exactly 64 hex chars, see
/// [`generate_ctrl_token`]).
pub fn constant_time_eq(a: &str, b: &str) -> bool {
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

/// Scans `$PATH` for an executable file named `bin`, without forking a
/// `which` subprocess. `$PATH`-search semantics only (no absolute-path
/// support): every caller here passes a bare command name.
pub fn which(bin: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| {
        let candidate = dir.join(bin);
        candidate
            .metadata()
            .map(|m| m.is_file() && is_executable(&m))
            .unwrap_or(false)
    })
}

fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.tmp");
        write_owner_only_file(&path, "secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret");
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
    fn percent_encode_round_trips_through_percent_decode() {
        let path = "/my-files/Reports/q3 report (final).pdf";
        assert_eq!(percent_decode(&percent_encode(path)), path);
    }

    #[test]
    fn extract_query_param_reads_a_value_from_the_request_line() {
        let req = "GET /route?path=%2Fhome%2Fuser HTTP/1.1\r\nHost: x\r\n";
        assert_eq!(
            extract_query_param(req, "path"),
            Some("/home/user".to_string())
        );
        assert_eq!(extract_query_param(req, "missing"), None);
    }

    #[test]
    fn extract_header_is_case_insensitive() {
        let req = "GET / HTTP/1.1\r\nX-Kio-Protondrive-Wizard-Token: abc123\r\n";
        assert_eq!(
            extract_header(req, "x-kio-protondrive-wizard-token"),
            Some("abc123")
        );
    }

    #[test]
    fn request_path_strips_the_query_string() {
        assert_eq!(request_path("GET /route?a=1&b=2 HTTP/1.1\r\n"), "/route");
    }

    #[test]
    fn request_method_reads_the_verb() {
        assert_eq!(request_method("OPTIONS /route HTTP/1.1\r\n"), "OPTIONS");
    }

    #[test]
    fn constant_time_eq_matches_regular_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }

    #[test]
    fn runtime_dir_prefers_xdg_runtime_dir_over_the_uid_fallback() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/9999");
        assert_eq!(runtime_dir(42), PathBuf::from("/run/user/9999"));
        std::env::remove_var("XDG_RUNTIME_DIR");
        assert_eq!(runtime_dir(42), PathBuf::from("/run/user/42"));
    }

    #[test]
    fn which_finds_a_binary_known_to_exist_in_this_test_environment() {
        // `sh` is POSIX-guaranteed to exist wherever this test runs.
        assert!(which("sh"));
        assert!(!which("definitely-not-a-real-binary-name-xyz"));
    }
}
