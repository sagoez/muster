use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A handle whose pause and resume always fail.
pub(crate) struct PauseFailHandle;

impl ProcessHandle for PauseFailHandle {
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

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported("signal failed".to_string()))
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported("signal failed".to_string()))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}
