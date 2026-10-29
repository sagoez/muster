use std::fmt::Debug;

use crate::domain::pty::PtyError;

/// Reads the visible screen of a process from wherever it is served. A read
/// may block, so a caller runs it off its own loop.
pub trait ScreenReader: Debug + Send + Sync {
    /// The process's visible screen, as text.
    ///
    /// # Errors
    /// Returns a `PtyError` when the screen cannot be reached or read.
    fn read_screen(&self) -> Result<String, PtyError>;
}
