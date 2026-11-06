use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

use super::fake_handle::FakeHandle;

/// A runner whose spawned processes never emit output and never exit on their
/// own; the test drives exits explicitly.
pub(crate) struct FakeRunner;

impl ProcessRunner for FakeRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(FakeHandle))
    }
}
