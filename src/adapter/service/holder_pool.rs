use std::{
    io,
    path::{Path, PathBuf},
};

use super::supervisor::HolderAdopter;
use crate::{
    adapter::holder::{HolderRecord, HolderRunner},
    domain::{port::ProcessRunner, value::ProcessName},
};

/// Every holder the service can reach: it launches new ones, adopts and
/// locates running ones, and tidies what failed launches left behind. One pool
/// serves every project's supervisor.
pub trait HolderPool: ProcessRunner + HolderAdopter + Send + Sync {
    /// The records of holders still running, and of dead holders whose child
    /// still runs; every other record is deleted.
    fn live_records(&self) -> Vec<HolderRecord>;

    /// The record files that cannot be read, which no supervisor can adopt.
    fn unreadable_records(&self) -> Vec<PathBuf>;

    /// Deletes the handover files holders that never started left behind and
    /// the sockets of holders that died, returning each one that could not be
    /// removed with why.
    ///
    /// # Errors
    /// Returns the I/O error from listing them.
    fn sweep_handover_files(&self) -> io::Result<Vec<(PathBuf, io::Error)>>;

    /// The log every run of the process `name` of `project` writes to.
    fn process_log(&self, project: &Path, name: &ProcessName) -> PathBuf;

    /// Removes the per-holder logs an earlier muster left behind.
    ///
    /// # Errors
    /// Returns the I/O error from removing them.
    fn remove_legacy_logs(&self) -> io::Result<()>;
}

impl HolderPool for HolderRunner {
    fn live_records(&self) -> Vec<HolderRecord> {
        HolderRunner::live_records(self)
    }

    fn unreadable_records(&self) -> Vec<PathBuf> {
        HolderRunner::unreadable_records(self)
    }

    fn sweep_handover_files(&self) -> io::Result<Vec<(PathBuf, io::Error)>> {
        let mut failures = HolderRunner::sweep_handover_files(self)?;
        failures.extend(self.sweep_orphan_sockets()?);
        Ok(failures)
    }

    fn process_log(&self, project: &Path, name: &ProcessName) -> PathBuf {
        self.dirs().process_log(project, name)
    }

    fn remove_legacy_logs(&self) -> io::Result<()> {
        self.dirs().remove_legacy_logs()
    }
}
