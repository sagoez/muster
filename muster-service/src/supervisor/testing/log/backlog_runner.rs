use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

use super::backlog_handle::BacklogHandle;

/// A runner whose processes printed [`BACKLOG`](crate::supervisor::testing::fixtures::BACKLOG)
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
