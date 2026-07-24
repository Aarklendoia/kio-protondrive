//! Daemon configuration, loaded from `~/.config/kio-protondrive/daemon.toml`.
//!
//! Phase 1 is hand-edited TOML only, one folder pair, no GUI/wizard — see
//! docs/DESIGN.md for the eventual multi-folder / bi-directional shape this
//! is expected to grow into.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::DaemonError;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Local folder to watch for new/changed files.
    pub local_path: PathBuf,
    /// Proton Drive folder new/changed files are uploaded into (e.g.
    /// "/my-files/Backups").
    pub remote_path: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, DaemonError> {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    /// `~/.config/kio-protondrive/daemon.toml`, per XDG_CONFIG_HOME (or its
    /// `~/.config` fallback).
    pub fn default_path() -> PathBuf {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".config")
            });
        config_home.join("kio-protondrive").join("daemon.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_a_minimal_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(
            &config_path,
            r#"
            local_path = "/home/user/Sync"
            remote_path = "/my-files/Backups"
            "#,
        )
        .unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.local_path, PathBuf::from("/home/user/Sync"));
        assert_eq!(config.remote_path, "/my-files/Backups");
    }

    #[test]
    fn load_fails_on_missing_file() {
        let err = Config::load(Path::new("/nonexistent/daemon.toml")).unwrap_err();
        assert!(matches!(err, DaemonError::Io(_)));
    }
}
