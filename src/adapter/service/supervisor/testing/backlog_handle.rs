use std::time::{Duration, SystemTime};

use super::fixtures::{BACKLOG, BACKLOG_RUN_START};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A handle whose run began at [`BACKLOG_RUN_START`] and logged [`BACKLOG`]
/// before it connected, as a holder reached after its child printed.
pub(crate) struct BacklogHandle;

impl ProcessHandle for BacklogHandle {
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

    fn output_end(&self) -> Option<OutputCursor> {
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

    fn run_start(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(BACKLOG_RUN_START))
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        Some((OutputCursor::new(BACKLOG_RUN_START), BACKLOG.to_vec()))
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
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
