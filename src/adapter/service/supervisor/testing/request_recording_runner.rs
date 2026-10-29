use super::{fake_handle::FakeHandle, spawn_recorder::SpawnRecorder};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A successful runner that retains each spawn request.
pub(crate) struct RequestRecordingRunner {
    pub(crate) recorder: SpawnRecorder,
}

impl ProcessRunner for RequestRecordingRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.recorder.requests.borrow_mut().push(request);
        Ok(Box::new(FakeHandle))
    }
}
