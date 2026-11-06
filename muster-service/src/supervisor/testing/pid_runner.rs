use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

use super::pid_handle::PidHandle;

/// A successful runner whose handles report a pid.
pub(crate) struct PidRunner;

impl ProcessRunner for PidRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(PidHandle))
    }
}
