use std::sync::Arc;

use crossbeam_channel::Receiver;
use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid, ProcessName, ProjectRoot},
    },
};

/// An adopter that runs on a worker, as the service's does, with no running
/// holders, whose log reads block until `gate` is dropped, as a log on a
/// stalled file system would.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct StuckLogAdopter {
    gate: Receiver<()>,
}

impl HolderAdopter for StuckLogAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
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
        Err(PtyError::System(String::new()))
    }

    fn log_tail(
        &self,
        _project: &ProjectRoot,
        _name: &ProcessName,
    ) -> Option<(OutputCursor, Vec<u8>)> {
        let _ = self.gate.recv();
        None
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}
