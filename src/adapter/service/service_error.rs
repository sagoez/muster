use std::{io, path::PathBuf};

use thiserror::Error;

use crate::domain::config::ConfigError;

/// Why the service could not start or keep running.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// The state directory, its lock or its socket could not be used.
    #[error("service i/o error: {0}")]
    Io(#[from] io::Error),
    /// No state directory is available on this platform.
    #[error("no state directory is available for the service")]
    NoStateDir,
    /// The socket path is in use by something this service cannot replace.
    #[error("could not bind the service socket {path}: {source}")]
    Bind {
        /// The socket that could not be bound.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// The settings every launch uses could not be read at start.
    #[error("could not read the settings: {0}")]
    Settings(#[source] ConfigError),
}
