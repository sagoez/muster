use std::{
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::Duration,
};

use crate::domain::{
    config::ConfigError,
    port::{RaisedEdits, RunHistory},
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// What the store refuses with while another writer holds it.
const BUSY: &str = "database is locked";
/// What the store refuses with when the write itself is wrong.
const BROKEN: &str = "constraint failed";

/// A run history and raised-edit store that refuses a set number of writes,
/// as busy (a database another writer holds) or broken (a write that can
/// never land), or takes each write only after a stall. Clones share what
/// they recorded.
#[derive(Clone)]
pub struct FailingHistory {
    /// Writes still to refuse, every write tried, and the processes whose
    /// end landed.
    state: Arc<Mutex<(u32, u32, Vec<ProcessName>)>>,
    /// Whether a refusal is busy rather than broken.
    busy: bool,
    /// How long each write blocks before it is answered.
    stall: Duration,
}

impl FailingHistory {
    /// A store that refuses its first `refusals` writes as busy.
    pub fn failing(refusals: u32) -> Self {
        Self::refusing(refusals, true, Duration::ZERO)
    }

    /// A store that refuses its first `refusals` writes as broken.
    pub fn broken(refusals: u32) -> Self {
        Self::refusing(refusals, false, Duration::ZERO)
    }

    /// A store that takes every write, each only after `stall`.
    pub fn stalling(stall: Duration) -> Self {
        Self::refusing(0, true, stall)
    }

    /// A store refusing its first `refusals` writes, as busy when `busy`,
    /// blocking each write for `stall`.
    fn refusing(refusals: u32, busy: bool, stall: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new((refusals, 0, Vec::new()))),
            busy,
            stall,
        }
    }

    /// The processes whose end landed, in order.
    pub fn recorded(&self) -> Vec<ProcessName> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .2
            .clone()
    }

    /// How many writes were tried, refused ones included.
    pub fn attempts(&self) -> u32 {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).1
    }

    /// Counts a write, refusing it while refusals remain.
    ///
    /// # Errors
    /// Returns a busy or a storage failure while refusals remain.
    fn attempt(&self, landed: Option<&ProcessName>) -> Result<(), ConfigError> {
        thread::sleep(self.stall);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1 += 1;
        if state.0 > 0 {
            state.0 -= 1;
            return Err(if self.busy {
                ConfigError::Busy(BUSY.into())
            } else {
                ConfigError::Storage(BROKEN.into())
            });
        }
        state.2.extend(landed.cloned());
        Ok(())
    }
}

impl RunHistory for FailingHistory {
    fn ended_runs(&self, _root: &ProjectRoot) -> Result<Vec<EndedRun>, ConfigError> {
        Ok(Vec::new())
    }

    fn record_end(&self, _root: &ProjectRoot, run: &EndedRun) -> Result<(), ConfigError> {
        self.attempt(Some(run.process()))
    }

    fn clear_end(&self, _root: &ProjectRoot, _process: &ProcessName) -> Result<(), ConfigError> {
        self.attempt(None)
    }
}

impl RaisedEdits for FailingHistory {
    fn raised_through(&self, _root: &ProjectRoot) -> Result<Option<EditId>, ConfigError> {
        Ok(None)
    }

    fn mark_raised(&self, _root: &ProjectRoot, _edit: EditId) -> Result<(), ConfigError> {
        self.attempt(None)
    }
}
