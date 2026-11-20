use std::sync::{Arc, Mutex};

use getset::Getters;
use muster_domain::{process::LogPattern, value::OutputCursor};
use typed_builder::TypedBuilder;

use crate::supervisor::testing::{StubHandle, fixtures::REPLAY_STREAM_START};

/// An adopting handle that records the patterns each replay asks it for in
/// `asked`, replays nothing, and streams from [`REPLAY_STREAM_START`].
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ReplayAskHandle {
    asked: Arc<Mutex<Vec<Vec<LogPattern>>>>,
}

impl StubHandle for ReplayAskHandle {
    fn run_start(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(0))
    }

    fn output_end(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(REPLAY_STREAM_START))
    }

    fn replay_matching(&mut self, patterns: &[LogPattern]) -> Option<OutputCursor> {
        self.asked.lock().unwrap().push(patterns.to_vec());
        Some(OutputCursor::new(REPLAY_STREAM_START))
    }
}
