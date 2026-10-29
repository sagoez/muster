use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};

use super::stop_signals::StopSignals;
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// A handle that distinguishes graceful termination from a hard kill.
pub(crate) struct StopSignalHandle {
    pub(crate) signals: Arc<StopSignals>,
}

impl ProcessHandle for StopSignalHandle {
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
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates.fetch_add(1, Ordering::SeqCst);
        *self.signals.termination.lock().unwrap() = Some((signal, grace));
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.signals.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
