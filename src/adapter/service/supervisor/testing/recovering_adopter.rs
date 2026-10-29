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

/// An adopter of `record` whose holder refuses the first adoption and serves
/// every later one, as a holder busy when the service started does; counts
/// the adoptions tried.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RecoveringAdopter {
    record: HolderRecord,
    attempts: Rc<Cell<usize>>,
}

impl HolderAdopter for RecoveringAdopter {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        Some(self.record.clone()).filter(|record| record.child_pid() == child_pid)
    }

    fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        self.locate(child_pid).filter(|_| self.attempts.get() > 0)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        let attempt = self.attempts.get();
        self.attempts.set(attempt + 1);
        if attempt == 0 {
            return Err(PtyError::System(REFUSAL.to_string()));
        }
        Ok(Box::new(FixedPidHandle::new(record.child_pid())))
    }
}
