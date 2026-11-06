use std::sync::Arc;

use getset::Getters;
use muster_domain::{
    port::{OutputSink, ProcessHandle},
    pty::PtyError,
    value::{OutputCursor, Pid},
};
use muster_protocol::holder::HolderRecord;
use typed_builder::TypedBuilder;

use super::fixtures::REFUSAL;
use crate::supervisor::HolderAdopter;

/// An adopter that runs on a worker, as the service's does, finds
/// `record` for its child's pid and fails every adoption.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct FailingWorkerAdopter {
    record: Option<HolderRecord>,
}

impl HolderAdopter for FailingWorkerAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.record
            .clone()
            .filter(|record| record.child_pid() == child_pid)
    }

    fn serving(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Err(PtyError::System(REFUSAL.to_string()))
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
