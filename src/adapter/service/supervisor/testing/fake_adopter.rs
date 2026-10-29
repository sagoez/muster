use super::pid_handle::PidHandle;
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::Pid,
    },
};

/// An adopter whose every holder answers with a pid-reporting handle.
pub(crate) struct FakeAdopter;

impl HolderAdopter for FakeAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(PidHandle))
    }
}
