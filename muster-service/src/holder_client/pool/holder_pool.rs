use std::{io, path::PathBuf};

use muster_domain::{
    port::ProcessRunner,
    value::{ProcessName, ProjectRoot},
};
use muster_protocol::holder::{HolderError, HolderRecord};

use crate::{HolderCensus, supervisor::HolderAdopter};

/// Every holder the service can reach: it launches new ones, adopts and
/// locates running ones, and tidies what failed launches left behind. One pool
/// serves every project's supervisor.
pub trait HolderPool: ProcessRunner + HolderAdopter + Send + Sync {
    /// The records worth adopting of `project`, or of every project when
    /// none, read once, with whose holders still run and which left their
    /// child's exit worked out once for every pass that takes it: of holders
    /// still running, of dead holders whose child still runs, and of holders
    /// that left their child's exit. Every other record of them is deleted.
    fn census(&self, project: Option<&ProjectRoot>) -> HolderCensus;

    /// The record files that cannot be read, which no supervisor can adopt.
    fn unreadable_records(&self) -> Vec<PathBuf>;

    /// Deletes the handover files holders that never started left behind,
    /// the sockets of holders `census` found dead and the exits left beside
    /// no record, returning each file, or the sockets' folder, that could
    /// not be swept, with why.
    fn sweep_handover_files(&self, census: &HolderCensus) -> Vec<(PathBuf, HolderError)>;

    /// Sweeps the folder an earlier muster bound holder sockets in, once the
    /// holders it serves are adopted, and removes it when empty, returning
    /// what could not be swept, with why.
    fn sweep_legacy_sockets(&self, census: &HolderCensus) -> Vec<(PathBuf, HolderError)>;

    /// Forgets the record of a holder that left its child's exit beside it,
    /// and that exit, which is never reported.
    ///
    /// # Errors
    /// Returns the I/O error from removing them.
    fn forget_stranded(&self, record: &HolderRecord) -> io::Result<()>;

    /// The log every run of the process `name` of `project` writes to.
    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf;

    /// Removes the per-holder logs an earlier muster left behind.
    ///
    /// # Errors
    /// Returns the I/O error from removing them.
    fn remove_legacy_logs(&self) -> io::Result<()>;
}
