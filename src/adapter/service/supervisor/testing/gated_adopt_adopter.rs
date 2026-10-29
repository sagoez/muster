use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crossbeam_channel::Receiver;
use getset::Getters;
use typed_builder::TypedBuilder;

use super::{gated_locate_adopter::LOCATE_GATE_LIMIT, kill_count_handle::KillCountHandle};
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid},
    },
};

/// An adopter that runs on a worker, as the service's does, whose holder
/// for `record`'s child serves, and whose every adoption blocks until `gate`
/// lets it through, as a holder slow to greet does, then succeeds and is
/// counted. Every kill or abandon of the handles it gives counts in `kills`.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct GatedAdoptAdopter {
    record: HolderRecord,
    gate: Receiver<()>,
    adoptions: Arc<AtomicUsize>,
    kills: Arc<AtomicUsize>,
}

impl HolderAdopter for GatedAdoptAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        Some(self.record.clone()).filter(|record| record.child_pid() == child_pid)
    }

    fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.locate(child_pid)
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        let _ = self.gate.recv_timeout(LOCATE_GATE_LIMIT);
        self.adoptions.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(
            KillCountHandle::builder()
                .kills(Arc::clone(&self.kills))
                .build(),
        ))
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
