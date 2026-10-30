use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use typed_builder::TypedBuilder;

use super::{
    recording_input::RecordingInput,
    server_testing::{FAKE_PID, RESIZE_REFUSED},
};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A live child whose PTY refuses every resize and that records a kill
/// and its input.
#[derive(TypedBuilder)]
pub(super) struct FakeChild {
    killed: Arc<AtomicBool>,
    written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessHandle for FakeChild {
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn screen_size(&self) -> Option<PtySize> {
        None
    }

    fn attached_clients(&self) -> u32 {
        0
    }

    fn started_at(&self) -> Option<SystemTime> {
        None
    }

    fn run_start(&self) -> Option<OutputCursor> {
        None
    }

    fn output_end(&self) -> Option<OutputCursor> {
        None
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.kill()
    }

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
        Some(Box::new(RecordingInput::new(Arc::clone(&self.written))))
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
