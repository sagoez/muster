use std::sync::Arc;

use super::fixtures::REFUSAL;
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::Pid,
    },
};

/// An adopter that runs on a worker, as the service's does, finds
/// `record` for its child's pid and fails every adoption.
#[derive(Clone)]
pub(crate) struct FailingWorkerAdopter {
    pub(crate) record: Option<HolderRecord>,
}

impl HolderAdopter for FailingWorkerAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.record
            .clone()
            .filter(|record| record.child_pid() == child_pid)
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Err(PtyError::System(REFUSAL.to_string()))
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
