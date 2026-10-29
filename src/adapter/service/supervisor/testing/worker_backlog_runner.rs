use std::sync::Arc;

use super::backlog_runner::BacklogRunner;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A [`BacklogRunner`] whose launches run on a worker, as a holder adoption
/// after a restart does, so a test can act while the backlog is in flight.
#[derive(Clone)]
pub(crate) struct WorkerBacklogRunner;

impl ProcessRunner for WorkerBacklogRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        BacklogRunner.spawn(request, sink)
    }

    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
