use std::sync::{Arc, Mutex, PoisonError};

use crate::domain::{
    config::ConfigError,
    port::{RaisedEdits, RunHistory},
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// What the store refuses with while it is busy.
const BUSY: &str = "database is locked";

/// A run history and raised-edit store that refuses a set number of writes,
/// as a database another writer holds would, then takes them. Clones share
/// what they recorded.
#[derive(Clone)]
pub struct FailingHistory {
    /// Writes still to refuse, every write tried, and the processes whose
    /// end landed.
    state: Arc<Mutex<(u32, u32, Vec<ProcessName>)>>,
}

impl FailingHistory {
    /// A store that refuses its first `refusals` writes.
    pub fn failing(refusals: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new((refusals, 0, Vec::new()))),
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
    /// Returns a storage failure while refusals remain.
    fn attempt(&self, landed: Option<&ProcessName>) -> Result<(), ConfigError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1 += 1;
        if state.0 > 0 {
            state.0 -= 1;
            return Err(ConfigError::Storage(BUSY.into()));
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
