use std::sync::{Arc, Mutex};

use super::recording_handle::RecordingHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose spawned processes record their input.
pub(crate) struct RecordingRunner {
    pub(crate) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessRunner for RecordingRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(RecordingHandle {
            written: self.written.clone(),
        }))
    }
}
