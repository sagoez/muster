use std::sync::{Arc, Mutex};

use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A handle that records everything written to its PTY.
pub(crate) struct RecordingHandle {
    pub(crate) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessHandle for RecordingHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.written.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}
