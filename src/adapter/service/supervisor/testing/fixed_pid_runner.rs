use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::fixed_pid_handle::FixedPidHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
    value::Pid,
};

/// A runner whose every process reports `pid`, counting its spawns.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct FixedPidRunner {
    pid: Pid,
    spawns: Arc<AtomicUsize>,
}

impl ProcessRunner for FixedPidRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FixedPidHandle::new(self.pid)))
    }
}
