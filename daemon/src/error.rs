use protondrive_core::cli::DriveError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    Config(#[from] toml::de::Error),
    #[error("could not serialize config: {0}")]
    ConfigWrite(#[from] toml::ser::Error),
    #[error(transparent)]
    Drive(#[from] DriveError),
    #[error("failed to watch {path}: {source}")]
    Watch {
        path: std::path::PathBuf,
        #[source]
        source: notify::Error,
    },
}

impl DaemonError {
    /// True when the CLI reported a missing/expired session — the caller
    /// should stop retrying for this cycle (every remaining file would fail
    /// identically) and prompt the user to re-authenticate, rather than
    /// treating this like any other per-file failure.
    pub fn is_authentication_error(&self) -> bool {
        matches!(self, DaemonError::Drive(DriveError::NotAuthenticated))
    }
}
