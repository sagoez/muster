use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use crate::domain::{port::ProcessInput, pty::PtyError};

/// Reported when a writer panicked mid-write, leaving the PTY input unusable.
const INPUT_POISONED: &str = "pty input writer is poisoned";

/// The PTY's input writer, shared so another thread can write while the
/// handle's owner keeps controlling the process.
#[derive(Clone)]
pub(super) struct PtyInput(pub(super) Arc<Mutex<Box<dyn Write + Send>>>);

impl ProcessInput for PtyInput {
    /// Writes and flushes `bytes` to the PTY.
    ///
    /// # Errors
    /// Returns a `PtyError` if the write fails or a writer panicked mid-write.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut writer = self
            .0
            .lock()
            .map_err(|_| PtyError::System(INPUT_POISONED.to_string()))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }
}
