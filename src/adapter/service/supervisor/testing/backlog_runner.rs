use super::backlog_handle::BacklogHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose processes printed [`BACKLOG`](super::fixtures::BACKLOG)
/// before their handle connected, and stream nothing after.
pub(crate) struct BacklogRunner;

impl ProcessRunner for BacklogRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(BacklogHandle))
    }
}
