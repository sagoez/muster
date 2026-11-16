use std::{io, path::PathBuf};

use muster_domain::{
    port::ProcessRunner,
    pty::ExitOutcome,
    value::{ProcessName, ProjectRoot},
};
use muster_protocol::holder::{HolderError, HolderRecord};

use crate::{HolderRunner, supervisor::HolderAdopter};

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
    /// the sockets of holders that died, returning each file, or the sockets'
    /// folder, that could not be swept, with why.
    fn sweep_handover_files(&self) -> Vec<(PathBuf, HolderError)>;

    /// Sweeps the folder an earlier muster bound holder sockets in, once the
    /// holders it serves are adopted, and removes it when empty, returning
    /// what could not be swept, with why.
    fn sweep_legacy_sockets(&self) -> Vec<(PathBuf, HolderError)>;

    /// The exits holders that could not be reached left beside their
    /// records, each with its record, which is removed.
    fn stranded_exits(&self) -> Vec<(HolderRecord, ExitOutcome)>;

    /// The log every run of the process `name` of `project` writes to.
    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf;

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

    fn sweep_handover_files(&self) -> Vec<(PathBuf, HolderError)> {
        let mut failures = HolderRunner::sweep_handover_files(self);
        failures.extend(self.sweep_orphan_sockets());
        failures
    }

    fn sweep_legacy_sockets(&self) -> Vec<(PathBuf, HolderError)> {
        HolderRunner::sweep_legacy_sockets(self)
    }

    fn stranded_exits(&self) -> Vec<(HolderRecord, ExitOutcome)> {
        HolderRunner::stranded_exits(self)
    }

    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
        self.dirs().process_log(project, name)
    }

    fn remove_legacy_logs(&self) -> io::Result<()> {
        self.dirs().remove_legacy_logs()
    }
}
