use std::sync::{Arc, atomic::AtomicUsize};

use super::connected_handle::ConnectedHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose launches run on a worker, as a holder runner's do; the
/// shared count is the kills its handles were sent.
#[derive(Clone, Default)]
pub(crate) struct WorkerRunner(pub(crate) Arc<AtomicUsize>);

impl ProcessRunner for WorkerRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(ConnectedHandle(Arc::clone(&self.0))))
    }

    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
