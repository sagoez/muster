use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

use super::logged_ahead_handle::LoggedAheadHandle;

/// A runner whose processes' logs run ahead of the output they delivered.
pub(crate) struct LoggedAheadRunner;

impl ProcessRunner for LoggedAheadRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(LoggedAheadHandle))
    }
}
