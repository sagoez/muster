use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A handle that records everything written to its PTY.
pub(crate) struct RecordingHandle {
    pub(crate) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessHandle for RecordingHandle {
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
