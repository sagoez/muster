use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use crossbeam_channel::Receiver;

use super::{
    holder_pool::HolderPool, runtime_testing::WAIT, stalled_handle::StalledHandle,
    supervisor::HolderAdopter,
};
use crate::{
    adapter::holder::HolderRecord,
    domain::{
        port::{OutputSink, ProcessHandle, ProcessRunner},
        pty::{PtyError, SpawnRequest},
        value::{Pid, ProcessName, ProjectRoot},
    },
};

/// A pool whose running holders are `records`, none of which it can
/// adopt. Every spawn is counted and answers through a holder at `socket`
/// that never replies. While `gate` is held, a launch or an adoption
/// waits, as one for a holder that never becomes ready does.
pub(super) struct FakePool {
    pub(super) records: Vec<HolderRecord>,
    pub(super) spawns: Arc<AtomicUsize>,
    pub(super) kills: Arc<AtomicUsize>,
    pub(super) socket: Option<PathBuf>,
    pub(super) gate: Option<Receiver<()>>,
    /// Times the legacy logs were removed.
    pub(super) legacy_removals: Arc<AtomicUsize>,
    /// The failure the next legacy log removal reports.
    pub(super) legacy_result: Mutex<Option<io::Error>>,
}

impl FakePool {
    /// A pool over `records` with no gate, whose holders serve at `socket`.
    pub(super) fn new(records: Vec<HolderRecord>, socket: Option<PathBuf>) -> Self {
        Self {
            records,
            spawns: Arc::default(),
            kills: Arc::default(),
            socket,
            gate: None,
            legacy_removals: Arc::default(),
            legacy_result: Mutex::new(None),
        }
    }

    /// This pool, waiting in every launch and adoption until `gate`
    /// opens or is dropped.
    pub(super) fn gated(self, gate: Receiver<()>) -> Self {
        Self {
            gate: Some(gate),
            ..self
        }
    }

    /// Waits, bounded by [`WAIT`], for the gate to open.
    fn wait_at_gate(&self) {
        if let Some(gate) = &self.gate {
            let _ = gate.recv_timeout(WAIT);
        }
    }
}

impl ProcessRunner for FakePool {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        self.wait_at_gate();
        Ok(Box::new(StalledHandle {
            socket: self.socket.clone(),
            kills: Arc::clone(&self.kills),
        }))
    }
}

impl HolderAdopter for FakePool {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.wait_at_gate();
        Err(PtyError::System("unreachable holder".to_string()))
    }
}

impl HolderPool for FakePool {
    fn live_records(&self) -> Vec<HolderRecord> {
        self.records.clone()
    }

    fn unreadable_records(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn sweep_handover_files(&self) -> io::Result<Vec<(PathBuf, io::Error)>> {
        Ok(Vec::new())
    }

    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
        project.as_ref().join(format!("{name}.log"))
    }

    fn remove_legacy_logs(&self) -> io::Result<()> {
        self.legacy_removals.fetch_add(1, Ordering::SeqCst);
        self.legacy_result
            .lock()
            .unwrap()
            .take()
            .map_or(Ok(()), Err)
    }
}
