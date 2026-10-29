use std::{cell::Cell, rc::Rc};

use super::{fixed_pid_handle::FixedPidHandle, fixtures::REFUSAL};
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::Pid,
    },
};

/// An adopter that finds `record` for its child's pid, and adopts it unless
/// it `refuses`, counting each adoption.
pub(crate) struct ReconnectingAdopter {
    pub(crate) record: Option<HolderRecord>,
    pub(crate) refuses: bool,
    pub(crate) adoptions: Rc<Cell<usize>>,
}

impl HolderAdopter for ReconnectingAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.record
            .clone()
            .filter(|record| record.child_pid() == child_pid)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if self.refuses {
            return Err(PtyError::System(REFUSAL.to_string()));
        }
        self.adoptions.set(self.adoptions.get() + 1);
        Ok(Box::new(FixedPidHandle(record.child_pid())))
    }
}
