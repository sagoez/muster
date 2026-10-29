use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::Receiver;
use getset::Getters;
use typed_builder::TypedBuilder;

use super::fixed_pid_handle::FixedPidHandle;
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid},
    },
};

/// The longest a gated `locate` blocks, so a test that runs it on the loop by
/// mistake fails rather than hangs.
pub(crate) const LOCATE_GATE_LIMIT: Duration = Duration::from_secs(5);

/// An adopter that runs on a worker, as the service's does, whose `locate`
/// blocks until `gate` lets it through, then finds `record` for its child's
/// pid; every adoption succeeds and is counted.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct GatedLocateAdopter {
    record: HolderRecord,
    gate: Receiver<()>,
    adoptions: Arc<AtomicUsize>,
}

impl HolderAdopter for GatedLocateAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        let _ = self.gate.recv_timeout(LOCATE_GATE_LIMIT);
        Some(self.record.clone()).filter(|record| record.child_pid() == child_pid)
    }

    fn serving(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.adoptions.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FixedPidHandle::new(record.child_pid())))
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
