use std::sync::{Arc, Mutex};

use crate::domain::{port::ProcessInput, pty::PtyError};

/// Records whatever is written to it.
pub(super) struct RecordingInput(pub(super) Arc<Mutex<Vec<u8>>>);

impl ProcessInput for RecordingInput {
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }
}
