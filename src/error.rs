use std::io;

use thiserror::Error;

use crate::{
    adapter::{holder::HolderError, service::ServiceError},
    domain::{config::ConfigError, pty::PtyError},
};

/// Top-level error type for muster.
///
/// Domain and adapter modules define their own error enums; this aggregates
/// them transparently so each error's `Display` speaks for itself.
#[derive(Debug, Error)]
pub enum MusterError {
    /// An I/O failure while driving the terminal or a child process.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A failure reading or writing muster's stored state.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A PTY / process-spawning failure.
    #[error(transparent)]
    Pty(#[from] PtyError),
    /// A process holder failure.
    #[error(transparent)]
    Holder(#[from] HolderError),
    /// A service failure.
    #[error(transparent)]
    Service(#[from] ServiceError),
}
