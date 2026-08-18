//! Daemon configuration, loaded from `~/.config/kio-protondrive/daemon.toml`.
//!
//! `daemon.toml` is optional in practice — every field defaults to
//! something sane, so [`Config::load_or_default`] is fine with the file not
//! existing at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

/// Used when `cache_retention_days` is unset — see [`Config::cache_retention`].
pub const DEFAULT_CACHE_RETENTION_DAYS: u32 = 30;

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

    /// How many days an opportunistically-cached file (see
    /// `core::cache`'s `cached_files` table, issue #60) can go unaccessed
    /// before `cache_eviction::evict_stale`'s periodic sweep reclaims it.
    /// Never applies to explicitly *pinned* files, which this setting has
    /// no effect on. `None` (same "field can be omitted" convention as
    /// `credentials_store`) means [`DEFAULT_CACHE_RETENTION_DAYS`] — see
    /// [`Self::cache_retention`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_retention_days: Option<u32>,
}

impl Config {
    /// [`Self::cache_retention_days`] as a [`Duration`], falling back to
    /// [`DEFAULT_CACHE_RETENTION_DAYS`] when unset.
    pub fn cache_retention(&self) -> Duration {
        let days = self
            .cache_retention_days
            .unwrap_or(DEFAULT_CACHE_RETENTION_DAYS);
        Duration::from_secs(u64::from(days) * 24 * 60 * 60)
    }

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
            cache_retention_days: Some(7),
        };

        config.save(&config_path).unwrap();
        let loaded = Config::load_or_default(&config_path).unwrap();

        assert_eq!(loaded.credentials_store, config.credentials_store);
        assert_eq!(loaded.cache_retention_days, config.cache_retention_days);
    }

    #[test]
    fn cache_retention_falls_back_to_the_default_when_unset() {
        let config = Config::default();
        assert_eq!(
            config.cache_retention(),
            Duration::from_secs(u64::from(DEFAULT_CACHE_RETENTION_DAYS) * 24 * 60 * 60)
        );
    }

    #[test]
    fn cache_retention_honors_an_explicit_value() {
        let config = Config {
            credentials_store: None,
            cache_retention_days: Some(7),
        };
        assert_eq!(
            config.cache_retention(),
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }
}
