use std::sync::Arc;

use super::{holder_pool::HolderPool, supervisor::HolderAdopter};
use crate::{
    adapter::holder::HolderRecord,
    domain::{
        port::{OutputSink, ProcessHandle, ProcessRunner},
        pty::{PtyError, SpawnRequest},
        value::{OutputCursor, Pid},
    },
};

/// One service-wide [`HolderPool`] handed to a supervisor as both its runner
/// and its adopter, so every project launches and adopts through the same
/// holders.
#[derive(Clone)]
pub struct SharedHolders(Arc<dyn HolderPool>);

impl SharedHolders {
    /// Shares `pool`.
    pub fn new(pool: Arc<dyn HolderPool>) -> Self {
        Self(pool)
    }
}

impl ProcessRunner for SharedHolders {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.0.spawn(request, sink)
    }

    /// A launch waits for its holder to serve, so it runs off the loop.
    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}

impl HolderAdopter for SharedHolders {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.0.locate(child_pid)
    }

    fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.0.serving(child_pid)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        since: Option<OutputCursor>,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.0.adopt(record, since, sink)
    }

    /// An adoption waits for the holder's screen, so it runs off the loop.
    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
