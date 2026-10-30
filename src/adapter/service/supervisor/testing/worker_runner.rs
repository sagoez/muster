use std::sync::{Arc, atomic::AtomicUsize};

use super::connected_handle::ConnectedHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose launches run on a worker, as a holder runner's do; the
/// shared count is the kills its handles were sent.
#[derive(Clone, Default)]
pub(crate) struct WorkerRunner(Arc<AtomicUsize>);

impl WorkerRunner {
    /// The kills its handles were sent.
    pub(crate) fn kills(&self) -> &Arc<AtomicUsize> {
        &self.0
    }
}

impl ProcessRunner for WorkerRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(ConnectedHandle::new(Arc::clone(&self.0))))
    }

    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
