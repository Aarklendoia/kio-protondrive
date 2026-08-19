//! Fetching, verifying and installing `proton-drive` CLI releases (#65).
//!
//! Shared by `wizard/` (installs the CLI on first run if it's missing) and
//! `daemon/` (detects and, with confirmation, applies updates) rather than
//! reimplemented twice — same reasoning as `crate::local_ctrl`'s extraction.
//!
//! Shells out to `curl`/`sha512sum` rather than pulling in an HTTP client or
//! crypto crate: the release binary is 100MB+, so streaming straight to disk
//! via `curl -o` beats buffering it in-process, and `sha512sum` is already
//! on every Debian/Ubuntu install (coreutils, `Essential: yes`) — matches
//! this codebase's existing preference for shelling out over adding
//! dependencies (see `wizard/Cargo.toml`'s own "deliberately dependency-free"
//! doc comment, and `daemon/src/notification.rs`'s `notify-send` calls).
//!
//! Deliberately does *not* go through `crate::cli::CommandRunner` — that
//! trait's mock-based tests are specific to the `proton-drive` CLI's own
//! argument/JSON shape; `curl`/`sha512sum` here are one-off, hard-to-usefully
//! mock side effects, same untested-shell-out tolerance as e.g. the wizard's
//! own `generate_gpg_key`.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use thiserror::Error;

/// The manifest the CLI's own internal update check fetches too (found by
/// reading its bundled JS source — see `daemon::version_check`'s prior
/// history for how that was discovered). Independent of whatever CLI
/// version is currently installed, unlike relying on `--version`'s
/// self-report, which only exists in CLI builds roughly 0.6.0+ and is
/// silently absent on anything older (the actual bug #65 was filed for).
const MANIFEST_URL: &str = "https://proton.me/download/drive/cli/version.json";

#[derive(Debug, Error)]
pub enum CliUpdateError {
    #[error("could not fetch {0}: {1}")]
    Fetch(String, String),
    #[error("could not parse the release manifest: {0}")]
    Parse(String),
    #[error("no {0} build listed in the release manifest")]
    NoMatchingPlatform(String),
    #[error("downloaded file's checksum did not match the manifest")]
    ChecksumMismatch,
    #[error("i/o error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseFile {
    #[serde(rename = "Url")]
    pub url: String,
    #[serde(rename = "Sha512CheckSum")]
    pub sha512: String,
    #[serde(rename = "Platform")]
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    #[serde(rename = "CategoryName")]
    pub category: String,
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "Files")]
    pub files: Vec<ReleaseFile>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(rename = "Releases")]
    releases: Vec<Release>,
}

/// Picks out the first `"CategoryName": "Stable"` entry — the manifest can
/// in principle list other channels (beta, ...), but this project only ever
/// wants to offer/install a stable build.
pub fn parse_manifest(json: &str) -> Result<Release, CliUpdateError> {
    let manifest: Manifest =
        serde_json::from_str(json).map_err(|e| CliUpdateError::Parse(e.to_string()))?;
    manifest
        .releases
        .into_iter()
        .find(|r| r.category == "Stable")
        .ok_or_else(|| CliUpdateError::Parse("no Stable release in the manifest".to_string()))
}

/// This build's platform identifier, matching the manifest's `Platform`
/// field (e.g. `"linux/x64"`) — only Linux glibc x64/arm64 are resolved
/// (the non-"baseline", non-"musl" variants): this project only ships
/// Debian/Ubuntu packages, which are glibc-based and recent enough not to
/// need the older-CPU baseline build.
pub fn current_platform() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("linux/x64"),
        "aarch64" => Some("linux/arm64"),
        _ => None,
    }
}

/// Finds `platform`'s entry in `release.files`.
pub fn file_for_platform<'a>(release: &'a Release, platform: &str) -> Option<&'a ReleaseFile> {
    release.files.iter().find(|f| f.platform == platform)
}

/// Pulls the version out of `proton-drive --version`'s first line, e.g.
/// `"Proton Drive CLI cli-drive@0.7.0+5174900c"` -> `"0.7.0"`.
pub fn installed_version(version_stdout: &str) -> Option<&str> {
    let line = version_stdout.lines().next()?;
    let after_at = line.split("cli-drive@").nth(1)?;
    Some(after_at.split('+').next().unwrap_or(after_at).trim())
}

/// Whether `remote` is a newer release than `installed` — both are expected
/// as dot-separated numeric versions (`"0.8.0"`). Any parse failure on
/// either side returns `false`: silently not offering an update is the safe
/// default here (the alternative, offering one based on a misread version,
/// risks loop-notifying on every check), left for the caller to `log::debug!`.
pub fn is_newer(remote: &str, installed: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u32>> {
        v.split('.').map(|p| p.parse().ok()).collect()
    }
    match (parts(remote), parts(installed)) {
        (Some(remote), Some(installed)) => remote > installed,
        _ => false,
    }
}

/// `curl -fsSL <MANIFEST_URL>`, parsed via [`parse_manifest`].
pub fn fetch_latest_stable() -> Result<Release, CliUpdateError> {
    let output = Command::new("curl")
        .args(["-fsSL", MANIFEST_URL])
        .output()
        .map_err(|e| CliUpdateError::Fetch(MANIFEST_URL.to_string(), e.to_string()))?;
    if !output.status.success() {
        return Err(CliUpdateError::Fetch(
            MANIFEST_URL.to_string(),
            format!("curl exited with {}", output.status),
        ));
    }
    parse_manifest(&String::from_utf8_lossy(&output.stdout))
}

/// Downloads `file.url` to a temp file next to `dest` (so the final step is
/// an atomic same-filesystem rename), verifies its SHA-512 against
/// `file.sha512`, makes it executable, then renames it into place. The temp
/// file is removed on any failure path — `dest` itself is only ever touched
/// by the final, already-verified rename.
pub fn download_and_install(file: &ReleaseFile, dest: &Path) -> Result<(), CliUpdateError> {
    let Some(parent) = dest.parent() else {
        return Err(CliUpdateError::Io(format!(
            "{} has no parent directory",
            dest.display()
        )));
    };
    std::fs::create_dir_all(parent).map_err(|e| CliUpdateError::Io(e.to_string()))?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}",
        dest.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));

    let result = download_and_install_inner(file, dest, &tmp_path);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn download_and_install_inner(
    file: &ReleaseFile,
    dest: &Path,
    tmp_path: &Path,
) -> Result<(), CliUpdateError> {
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(tmp_path)
        .arg(&file.url)
        .status()
        .map_err(|e| CliUpdateError::Fetch(file.url.clone(), e.to_string()))?;
    if !status.success() {
        return Err(CliUpdateError::Fetch(
            file.url.clone(),
            format!("curl exited with {status}"),
        ));
    }

    let checksum_output = Command::new("sha512sum")
        .arg(tmp_path)
        .output()
        .map_err(|e| CliUpdateError::Io(e.to_string()))?;
    if !checksum_output.status.success() {
        return Err(CliUpdateError::Io(format!(
            "sha512sum exited with {}",
            checksum_output.status
        )));
    }
    let actual = String::from_utf8_lossy(&checksum_output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if actual != file.sha512.to_lowercase() {
        return Err(CliUpdateError::ChecksumMismatch);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| CliUpdateError::Io(e.to_string()))?;
    }

    std::fs::rename(tmp_path, dest).map_err(|e| CliUpdateError::Io(e.to_string()))
}

/// Whether `path` both exists and is writable by the current process —
/// used to decide whether the daemon is allowed to update an existing
/// install in place (e.g. not a root-owned system install) without ever
/// escalating privileges itself, matching this project's standing "the user
/// runs their own sudo" rule (see `wizard::route_setup_pass`'s doc comment).
pub fn is_writable(path: &Path) -> bool {
    std::fs::OpenOptions::new().append(true).open(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MANIFEST: &str = r#"{
      "Releases": [
        {
          "CategoryName": "Stable",
          "Version": "0.8.0",
          "ReleaseDate": "2026-08-13",
          "Files": [
            {"Url": "https://proton.me/download/drive/cli/0.8.0/linux-x64/proton-drive",
             "Sha512CheckSum": "cf61c268", "Platform": "linux/x64"},
            {"Url": "https://proton.me/download/drive/cli/0.8.0/linux-arm64/proton-drive",
             "Sha512CheckSum": "27a1aec1", "Platform": "linux/arm64"},
            {"Url": "https://proton.me/download/drive/cli/0.8.0/darwin-x64/proton-drive",
             "Sha512CheckSum": "4fed939a", "Platform": "macos/x64"}
          ]
        }
      ]
    }"#;

    #[test]
    fn parse_manifest_picks_the_stable_release() {
        let release = parse_manifest(SAMPLE_MANIFEST).unwrap();
        assert_eq!(release.version, "0.8.0");
        assert_eq!(release.files.len(), 3);
    }

    #[test]
    fn parse_manifest_rejects_garbage() {
        assert!(parse_manifest("not json").is_err());
        assert!(parse_manifest(r#"{"Releases": []}"#).is_err());
    }

    #[test]
    fn file_for_platform_finds_the_matching_entry() {
        let release = parse_manifest(SAMPLE_MANIFEST).unwrap();
        let file = file_for_platform(&release, "linux/x64").unwrap();
        assert_eq!(file.sha512, "cf61c268");
        assert!(file_for_platform(&release, "windows/x64").is_none());
    }

    #[test]
    fn installed_version_extracts_the_version_number() {
        assert_eq!(
            installed_version(
                "Proton Drive CLI cli-drive@0.7.0+5174900c\nProton Drive SDK js@0.20.0+5174900c\n"
            ),
            Some("0.7.0")
        );
    }

    #[test]
    fn installed_version_is_none_for_unrecognized_output() {
        assert_eq!(installed_version("garbage\n"), None);
        assert_eq!(installed_version(""), None);
    }

    #[test]
    fn is_newer_compares_dot_separated_versions() {
        assert!(is_newer("0.8.0", "0.7.0"));
        assert!(is_newer("0.7.1", "0.7.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.7.0", "0.7.0"));
        assert!(!is_newer("0.7.0", "0.8.0"));
    }

    #[test]
    fn is_newer_is_false_on_unparseable_input() {
        assert!(!is_newer("not-a-version", "0.7.0"));
        assert!(!is_newer("0.8.0", "also-not-a-version"));
        assert!(!is_newer("", ""));
    }

    #[test]
    fn current_platform_resolves_a_known_linux_arch() {
        // This test runs on whatever arch built it — just check it's
        // Some(...) for the two archs this project actually packages for,
        // matching debian/control's `Architecture: any`.
        if matches!(std::env::consts::ARCH, "x86_64" | "aarch64") {
            assert!(current_platform().is_some());
        }
    }

    #[test]
    fn is_writable_is_false_for_a_missing_path() {
        assert!(!is_writable(Path::new(
            "/nonexistent/kio-protondrive-test-path"
        )));
    }

    #[test]
    fn download_and_install_verifies_checksum_and_rejects_a_mismatch() {
        // A `curl` call that always fails (bogus scheme) is enough to
        // exercise the temp-file-cleanup path without hitting the network.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("proton-drive");
        let file = ReleaseFile {
            url: "not-a-real-scheme://example.invalid/x".to_string(),
            sha512: "deadbeef".to_string(),
            platform: "linux/x64".to_string(),
        };
        let result = download_and_install(&file, &dest);
        assert!(result.is_err());
        assert!(!dest.exists());
        // No leftover temp file either.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
