use std::{io, path::PathBuf};

use thiserror::Error;

use crate::{
    adapter::{no_state_dir::NoStateDir, private_dir::RUNTIME_DIR_ENV},
    domain::config::ConfigError,
};

/// Why the service could not start or keep running.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// The state directory, its lock or its socket could not be used.
    #[error("service i/o error: {0}")]
    Io(#[from] io::Error),
    /// No state directory is available on this platform.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// The socket path is in use by something this service cannot replace.
    #[error("could not bind the service socket {path}: {source}")]
    Bind {
        /// The socket that could not be bound.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// Where the service listens could not be recorded for its clients.
    #[error("could not record the service socket {path} in the state directory: {source}")]
    Publish {
        /// The socket the service bound.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// A path the service binds or listens at does not fit in a Unix socket
    /// address.
    #[error(
        "the socket path {path} is too long for a Unix socket; set {variable} to a shorter folder",
        variable = RUNTIME_DIR_ENV
    )]
    SocketPathTooLong {
        /// The path that does not fit.
        path: PathBuf,
    },
    /// The settings every launch uses could not be read at start.
    #[error("could not read the settings: {0}")]
    Settings(#[source] ConfigError),
}
