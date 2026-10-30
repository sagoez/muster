use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::domain::{
    occupancy::{ReportStamp, ShellId, ShellRecord, ShellRegistryError},
    port::ShellRegistry,
    value::Pid,
};

/// Shells and their records held in memory and shared with the test, which
/// opens and closes shells as a user would.
#[derive(Clone, Default)]
pub(super) struct FakeShells {
    /// The shells that run now.
    live: Arc<Mutex<BTreeSet<ShellId>>>,
    /// The record kept under each pid.
    records: Arc<Mutex<BTreeMap<Pid, ShellRecord>>>,
    /// Every shell forgotten, in order.
    forgotten: Arc<Mutex<Vec<ShellId>>>,
}

impl FakeShells {
    /// Starts `shell`.
    pub(super) fn open(&self, shell: ShellId) {
        self.live.lock().unwrap().insert(shell);
    }

    /// Ends `shell`.
    pub(super) fn close(&self, shell: &ShellId) {
        self.live.lock().unwrap().remove(shell);
    }

    /// Keeps `record` under its pid, as a report would have left it.
    pub(super) fn put(&self, record: ShellRecord) {
        self.records
            .lock()
            .unwrap()
            .insert(record.shell().pid(), record);
    }

    /// Every shell forgotten so far, in order.
    pub(super) fn forgotten(&self) -> Vec<ShellId> {
        self.forgotten.lock().unwrap().clone()
    }
}

impl ShellRegistry for FakeShells {
    fn identify(&self, pid: Pid) -> Option<ShellId> {
        self.live
            .lock()
            .unwrap()
            .iter()
            .find(|shell| shell.pid() == pid)
            .copied()
    }

    fn is_live(&self, shell: &ShellId) -> bool {
        self.live.lock().unwrap().contains(shell)
    }

    fn records(&self) -> Vec<ShellRecord> {
        self.records.lock().unwrap().values().cloned().collect()
    }

    fn record(&self, shell: &ShellId) -> Option<ShellRecord> {
        self.records
            .lock()
            .unwrap()
            .get(&shell.pid())
            .filter(|record| record.shell() == shell)
            .cloned()
    }

    fn save_if_newer(&self, record: &ShellRecord) -> Result<bool, ShellRegistryError> {
        if self
            .record(record.shell())
            .is_some_and(|stored| !record.stamp().supersedes(stored.stamp()))
        {
            return Ok(false);
        }
        self.put(record.clone());
        Ok(true)
    }

    fn amend(&self, record: &ShellRecord) -> Result<bool, ShellRegistryError> {
        if self
            .record(record.shell())
            .is_none_or(|stored| stored.stamp() != record.stamp())
        {
            return Ok(false);
        }
        self.put(record.clone());
        Ok(true)
    }

    fn deliver(
        &self,
        _shell: &ShellId,
        _stamp: ReportStamp,
        _lines: &[String],
    ) -> Result<(), ShellRegistryError> {
        Ok(())
    }

    fn forget(&self, shell: &ShellId) {
        let mut records = self.records.lock().unwrap();
        if records
            .get(&shell.pid())
            .is_some_and(|record| record.shell() == shell)
        {
            records.remove(&shell.pid());
        }
        self.forgotten.lock().unwrap().push(*shell);
    }

    fn notes_dir(&self) -> PathBuf {
        PathBuf::new()
    }
}
