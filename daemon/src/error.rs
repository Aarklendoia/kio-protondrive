use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    Config(#[from] toml::de::Error),
    #[error("journal database error: {0}")]
    Journal(#[from] rusqlite::Error),
    #[error(transparent)]
    Drive(#[from] protondrive_core::cli::DriveError),
    #[error("failed to watch {path}: {source}")]
    Watch {
        path: std::path::PathBuf,
        #[source]
        source: notify::Error,
    },
}
