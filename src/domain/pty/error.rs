use std::path::PathBuf;

use thiserror::Error;

use super::PtySize;

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
    /// A terminal size outside the PTY bounds.
    #[error(
        "terminal size {rows}x{cols} is outside {min_rows}x{min_cols} to {max_rows}x{max_cols} (rows x columns)",
        min_rows = PtySize::MIN_ROWS,
        min_cols = PtySize::MIN_COLS,
        max_rows = PtySize::MAX_ROWS,
        max_cols = PtySize::MAX_COLS
    )]
    SizeOutOfBounds {
        /// The refused rows.
        rows: u16,
        /// The refused columns.
        cols: u16,
    },
}
