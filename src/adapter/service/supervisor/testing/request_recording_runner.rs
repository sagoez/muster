use getset::Getters;
use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};
use typed_builder::TypedBuilder;

use super::{fake_handle::FakeHandle, spawn_recorder::SpawnRecorder};

/// A successful runner that retains each spawn request.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RequestRecordingRunner {
    recorder: SpawnRecorder,
}

impl ProcessRunner for RequestRecordingRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.recorder.requests().borrow_mut().push(request);
        Ok(Box::new(FakeHandle))
    }
}
