use std::sync::Arc;

use super::failing_runner::FailingRunner;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose launches run on a worker and always fail.
#[derive(Clone)]
pub(crate) struct FailingWorkerRunner;

impl ProcessRunner for FailingWorkerRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        FailingRunner.spawn(request, sink)
    }

    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
