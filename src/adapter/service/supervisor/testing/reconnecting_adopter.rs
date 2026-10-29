use std::{cell::Cell, rc::Rc};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{fixed_pid_handle::FixedPidHandle, fixtures::REFUSAL};
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid},
    },
};

/// An adopter that finds `record` for its child's pid, and adopts it unless
/// it `refuses`, counting each adoption.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ReconnectingAdopter {
    record: Option<HolderRecord>,
    refuses: bool,
    adoptions: Rc<Cell<usize>>,
}

impl HolderAdopter for ReconnectingAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.record
            .clone()
            .filter(|record| record.child_pid() == child_pid)
    }

    fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.locate(child_pid).filter(|_| !self.refuses)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if self.refuses {
            return Err(PtyError::System(REFUSAL.to_string()));
        }
        self.adoptions.set(self.adoptions.get() + 1);
        Ok(Box::new(FixedPidHandle::new(record.child_pid())))
    }
}
