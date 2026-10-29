use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::{
    recording_input::RecordingInput,
    server_testing::{FAKE_PID, RESIZE_REFUSED},
};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A live child whose PTY refuses every resize and that records a kill
/// and its input.
pub(super) struct FakeChild {
    pub(super) killed: Arc<AtomicBool>,
    pub(super) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessHandle for FakeChild {
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn process_id(&self) -> Option<Pid> {
        Some(Pid::try_new(FAKE_PID).unwrap())
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        Some(Box::new(RecordingInput(Arc::clone(&self.written))))
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Err(PtyError::System(RESIZE_REFUSED.to_string()))
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
