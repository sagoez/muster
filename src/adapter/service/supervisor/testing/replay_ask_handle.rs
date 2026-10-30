use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::fixtures::REPLAY_STREAM_START;
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::{LogPattern, StopSignal},
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// An adopting handle that records the patterns each replay asks it for in
/// `asked`, replays nothing, and streams from [`REPLAY_STREAM_START`].
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ReplayAskHandle {
    asked: Arc<Mutex<Vec<Vec<LogPattern>>>>,
}

impl ProcessHandle for ReplayAskHandle {
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
        Some(OutputCursor::new(0))
    }

    fn output_end(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(REPLAY_STREAM_START))
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    fn replay_matching(&mut self, patterns: &[LogPattern]) -> Option<OutputCursor> {
        self.asked.lock().unwrap().push(patterns.to_vec());
        Some(OutputCursor::new(REPLAY_STREAM_START))
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
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
