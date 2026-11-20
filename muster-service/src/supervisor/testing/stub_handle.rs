use std::time::{Duration, SystemTime};

use muster_domain::{
    port::{ProcessHandle, ProcessInput},
    process::{LogPattern, StopSignal},
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

use super::stubbed_handle::StubbedHandle;

/// A test process handle: every call is a quiet success by default, so a
/// fake overrides only what its test watches. [`StubHandle::boxed`] runs it
/// as a [`ProcessHandle`].
pub(crate) trait StubHandle: Send + Sized + 'static {
    /// The child's pid; none by default.
    fn process_id(&self) -> Option<Pid> {
        None
    }

    /// A writer onto the child's input; none by default.
    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    /// Where the child's screen is read; kept by the supervisor by default.
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    /// The screen the child had when the handle connected; none by default.
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        None
    }

    /// The size of that screen; none by default.
    fn screen_size(&self) -> Option<PtySize> {
        None
    }

    /// The clients attached when the handle connected; none by default.
    fn attached_clients(&self) -> u32 {
        0
    }

    /// When the child started, if before the handle; none by default.
    fn started_at(&self) -> Option<SystemTime> {
        None
    }

    /// Where the current run begins in the log; none by default.
    fn run_start(&self) -> Option<OutputCursor> {
        None
    }

    /// Where the log ends now; none by default.
    fn output_end(&self) -> Option<OutputCursor> {
        None
    }

    /// Output logged before the handle connected; none by default.
    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    /// Replays the run's lines `_patterns` match; replays nothing by default.
    fn replay_matching(&mut self, _patterns: &[LogPattern]) -> Option<OutputCursor> {
        None
    }

    /// Acknowledges the child's exit.
    ///
    /// # Errors
    /// None by default.
    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    /// Writes `_bytes` to the child's input.
    ///
    /// # Errors
    /// None by default.
    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    /// Resizes the child's PTY to `_size`.
    ///
    /// # Errors
    /// None by default.
    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    /// Suspends the child.
    ///
    /// # Errors
    /// None by default.
    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    /// Resumes the child.
    ///
    /// # Errors
    /// None by default.
    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    /// Asks the child to stop with `_signal` within `_grace`; kills it by
    /// default.
    ///
    /// # Errors
    /// Whatever [`StubHandle::kill`] returns, by default.
    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.kill()
    }

    /// Kills the child.
    ///
    /// # Errors
    /// None by default.
    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    /// This stub as a boxed [`ProcessHandle`], whose abandon kills it.
    fn boxed(self) -> Box<dyn ProcessHandle> {
        Box::new(StubbedHandle::new(self))
    }
}
