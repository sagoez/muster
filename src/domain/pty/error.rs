use std::path::PathBuf;

use thiserror::Error;

/// Errors from spawning or interacting with a process under a PTY.
#[derive(Debug, Error)]
pub enum PtyError {
    /// An I/O failure reading from or writing to the PTY.
    #[error("pty i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The underlying PTY system reported a failure.
    #[error("pty operation failed: {0}")]
    System(String),
    /// The configured working directory is missing or not a directory.
    #[error("working directory is missing or not a directory: {0}")]
    InvalidWorkingDir(PathBuf),
    /// The requested operation is not supported here (e.g. process suspension
    /// on a non-Unix platform, or a signal with no target pid).
    #[error("operation not supported: {0}")]
    Unsupported(String),
}
