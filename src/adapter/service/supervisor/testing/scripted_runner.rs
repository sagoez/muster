use std::{cell::RefCell, collections::VecDeque};

use super::fake_handle::FakeHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// What a [`ScriptedRunner`]'s failed spawns report.
const SPAWN_FAILED: &str = "spawn failed";

/// A runner whose spawns succeed or fail in a scripted order, failing once
/// the script runs out.
pub(crate) struct ScriptedRunner {
    outcomes: RefCell<VecDeque<bool>>,
}

impl ScriptedRunner {
    /// A runner whose spawns succeed where `outcomes` is true, in order.
    pub(crate) fn new(outcomes: impl IntoIterator<Item = bool>) -> Self {
        Self {
            outcomes: RefCell::new(outcomes.into_iter().collect()),
        }
    }
}

impl ProcessRunner for ScriptedRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if self.outcomes.borrow_mut().pop_front().unwrap_or(false) {
            Ok(Box::new(FakeHandle))
        } else {
            Err(PtyError::System(SPAWN_FAILED.to_string()))
        }
    }
}
