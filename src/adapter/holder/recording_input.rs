use std::sync::{Arc, Mutex};

use crate::domain::{port::ProcessInput, pty::PtyError};

/// Records whatever is written to it.
pub(super) struct RecordingInput(Arc<Mutex<Vec<u8>>>);

impl RecordingInput {
    /// An input appending what it is written to `written`.
    pub(super) fn new(written: Arc<Mutex<Vec<u8>>>) -> Self {
        Self(written)
    }
}

impl ProcessInput for RecordingInput {
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }
}
