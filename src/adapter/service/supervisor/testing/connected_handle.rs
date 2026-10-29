use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use super::fixtures::FIRST_SCREEN;
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A handle that connected to a screen already showing [`FIRST_SCREEN`],
/// which its log holds too, counting the kills it is sent.
pub(crate) struct ConnectedHandle(pub(super) Arc<AtomicUsize>);

impl ProcessHandle for ConnectedHandle {
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

    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        Some(FIRST_SCREEN.to_vec())
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        Some((OutputCursor::new(0), FIRST_SCREEN.to_vec()))
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
