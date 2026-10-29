use super::fixtures::fake_launcher_pid;
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A handle that reports a pid, like the real PTY handle does.
pub(crate) struct PidHandle;

impl ProcessHandle for PidHandle {
    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn process_id(&self) -> Option<Pid> {
        Some(fake_launcher_pid())
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
