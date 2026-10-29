use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::fixed_pid_handle::FixedPidHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
    value::Pid,
};

/// A runner whose every process reports `pid`, counting its spawns.
pub(crate) struct FixedPidRunner {
    pub(crate) pid: Pid,
    pub(crate) spawns: Arc<AtomicUsize>,
}

impl ProcessRunner for FixedPidRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FixedPidHandle(self.pid)))
    }
}
