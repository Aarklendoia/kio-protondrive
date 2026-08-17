//! Daemon configuration, loaded from `~/.config/kio-protondrive/daemon.toml`.
//!
//! Since #30's pinned-cache design replaced the old configured-folder-pair
//! sync (no more `local_path`/`remote_path` to pick), the only setting
//! left is `credentials_store` — which already has a sane default. That
//! makes `daemon.toml` optional in practice: [`Config::load_or_default`]
//! is fine with it not existing at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Overrides the `proton-drive` CLI's `PROTON_DRIVE_CREDENTIALS_STORE`
    /// (see README's "Credential persistence" section for the trade-offs
    /// between `unsafe_file`/`keychain`/`pass`) — `None` (the field can be
    /// omitted entirely from the TOML, or the whole file can be missing)
    /// leaves the systemd unit's own
    /// `Environment=PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file` default in
    /// effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_store: Option<String>,
}

impl Config {
    /// Loads from `path`, or a default (empty) config if the file simply
    /// doesn't exist — `daemon.toml` is optional now (see the module doc).
    /// A malformed *existing* file still surfaces as a real error, though.
    pub fn load_or_default(path: &Path) -> Result<Self, DaemonError> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Ok(toml::from_str(&raw)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Writes this config to `path` as TOML, creating its parent directory
    /// if needed — the wizard's counterpart to `load_or_default`.
    pub fn save(&self, path: &Path) -> Result<(), DaemonError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
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
    fn load_or_default_returns_defaults_when_the_file_is_missing() {
        let config = Config::load_or_default(Path::new("/nonexistent/daemon.toml")).unwrap();
        assert_eq!(config.credentials_store, None);
    }

    #[test]
    fn load_or_default_parses_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(&config_path, "credentials_store = \"pass\"\n").unwrap();

        let config = Config::load_or_default(&config_path).unwrap();
        assert_eq!(config.credentials_store, Some("pass".to_string()));
    }

    #[test]
    fn load_or_default_still_errors_on_a_malformed_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(&config_path, "not valid toml [[[").unwrap();

        let err = Config::load_or_default(&config_path).unwrap_err();
        assert!(matches!(err, DaemonError::Config(_)));
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nested").join("daemon.toml");
        let config = Config {
            credentials_store: Some("pass".to_string()),
        };

        config.save(&config_path).unwrap();
        let loaded = Config::load_or_default(&config_path).unwrap();

        assert_eq!(loaded.credentials_store, config.credentials_store);
    }
}
