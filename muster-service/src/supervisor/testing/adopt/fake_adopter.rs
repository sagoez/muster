use muster_domain::{
    port::{OutputSink, ProcessHandle},
    pty::PtyError,
    value::{OutputCursor, Pid},
};
use muster_protocol::holder::HolderRecord;

use crate::supervisor::{HolderAdopter, testing::pid::pid_handle::PidHandle};

/// An adopter whose every holder answers with a pid-reporting handle.
pub(crate) struct FakeAdopter;

impl HolderAdopter for FakeAdopter {
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
        Ok(Box::new(PidHandle))
    }
}
