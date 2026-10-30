use super::pause_fail_handle::PauseFailHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles fail every pause and resume, modelling a signal that
/// cannot be delivered.
pub(crate) struct PauseFailRunner;

impl ProcessRunner for PauseFailRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(PauseFailHandle))
    }
}
