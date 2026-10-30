use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use crossbeam_channel::Receiver;
use getset::{Getters, MutGetters};

use super::{
    holder_pool::HolderPool, runtime_testing::WAIT, stalled_handle::StalledHandle,
    supervisor::HolderAdopter,
};
use crate::{
    adapter::holder::HolderRecord,
    domain::{
        port::{OutputSink, ProcessHandle, ProcessRunner},
        pty::{ProcessOutput, PtyError, SpawnRequest},
        value::{OutputCursor, Pid, ProcessName, ProjectRoot},
    },
};

/// Why a spawned handle's output stream broke.
pub(super) const BROKEN_STREAM: &str = "the holder stream broke";

/// A pool whose running holders are `records`, none of which it can
/// adopt. Every spawn is counted and answers through a holder at `socket`
/// that never replies. While `gate` is held, a launch or an adoption
/// waits, as one for a holder that never becomes ready does.
#[derive(Getters, MutGetters)]
pub(super) struct FakePool {
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    records: Vec<HolderRecord>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    spawns: Arc<AtomicUsize>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    kills: Arc<AtomicUsize>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    socket: Option<PathBuf>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    gate: Option<Receiver<()>>,
    /// The child pid every spawned handle reports, when one is given.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    pid: Option<Pid>,
    /// Whether every spawned handle's output stream breaks at once.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    breaks_streams: bool,
    /// Held, a look for a holder waits until it opens or is dropped.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    locate_gate: Option<Receiver<()>>,
    /// Times the legacy logs were removed.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    legacy_removals: Arc<AtomicUsize>,
    /// The failure the next legacy log removal reports.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    legacy_result: Mutex<Option<io::Error>>,
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
            pid: None,
            breaks_streams: false,
            locate_gate: None,
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

    /// This pool, whose every spawn runs the child `pid`, and whose every
    /// look for a holder waits until `locate_gate` opens or is dropped, then
    /// finds none.
    pub(super) fn running_as(self, pid: Pid, locate_gate: Receiver<()>) -> Self {
        Self {
            pid: Some(pid),
            locate_gate: Some(locate_gate),
            ..self
        }
    }

    /// This pool, whose every spawned handle's output stream breaks at once,
    /// so the service looks for its holder again.
    pub(super) fn breaking_streams(self) -> Self {
        Self {
            breaks_streams: true,
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
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        self.wait_at_gate();
        if self.breaks_streams {
            sink.send(ProcessOutput::Unreadable(BROKEN_STREAM.to_string()));
        }
        Ok(Box::new(
            StalledHandle::builder()
                .socket(self.socket.clone())
                .kills(Arc::clone(&self.kills))
                .pid(self.pid)
                .build(),
        ))
    }
}

impl HolderAdopter for FakePool {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        if let Some(gate) = &self.locate_gate {
            let _ = gate.recv_timeout(WAIT);
        }
        None
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
