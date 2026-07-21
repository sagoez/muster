use thiserror::Error;

use crate::domain::{config::ConfigError, pty::PtyError};

/// Top-level error type for muster.
///
/// Domain and adapter modules define their own error enums; this aggregates
/// them transparently so each error's `Display` speaks for itself.
#[derive(Debug, Error)]
pub enum MusterError {
    /// An I/O failure while driving the terminal or a child process.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// A workspace configuration failure.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A PTY / process-spawning failure.
    #[error(transparent)]
    Pty(#[from] PtyError),
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, MusterError>;
