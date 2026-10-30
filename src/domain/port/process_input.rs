use crate::domain::pty::PtyError;

/// The input half of a process's terminal, writable from a thread other than
/// the one controlling the process.
pub trait ProcessInput: Send {
    /// Writes raw input bytes to the process's PTY, blocking while it is full.
    ///
    /// # Errors
    /// Returns a `PtyError` if the write fails.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError>;
}
