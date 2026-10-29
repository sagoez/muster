use super::fixtures::{BACKLOG, BACKLOG_RUN_START};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A handle whose run began at [`BACKLOG_RUN_START`] and logged [`BACKLOG`]
/// before it connected, as a holder reached after its child printed.
pub(crate) struct BacklogHandle;

impl ProcessHandle for BacklogHandle {
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
