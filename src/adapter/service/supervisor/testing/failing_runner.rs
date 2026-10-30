use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose every spawn fails, modelling a persistent PTY or resource
/// failure.
pub(crate) struct FailingRunner;

impl ProcessRunner for FailingRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Err(PtyError::System("spawn failed".to_string()))
    }
}
