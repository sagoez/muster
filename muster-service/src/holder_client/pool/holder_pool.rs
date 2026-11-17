use std::{io, path::PathBuf};

use muster_domain::{
    port::ProcessRunner,
    value::{ProcessName, ProjectRoot},
};
use muster_protocol::holder::{HolderRecord, RecordScan};

use crate::{holder_client::pool::sweep_failure::SweepFailure, supervisor::HolderAdopter};

/// Every holder the service can reach: it launches new ones, adopts and
/// locates running ones, and tidies what failed launches left behind. One pool
/// serves every project's supervisor.
pub trait HolderPool: ProcessRunner + HolderAdopter + Send + Sync {
    /// The records of `project`, or of every project when none, read once,
    /// with whose holders still serve and which left their child's exit
    /// worked out once for every pass that takes it: it keeps the records of
    /// holders still serving, of dead holders whose child still runs, and of
    /// holders that left their child's exit, and every other record of them
    /// is deleted. The record files that cannot be read, which no supervisor
    /// can adopt, come with it. A service start reads this once and hands it
    /// to every sweep.
    fn scan_records(&self, project: Option<&ProjectRoot>) -> RecordScan;

    /// Sweeps the sockets' folder, listing it once: deletes the handover
    /// files holders that never started left behind and the sockets of
    /// holders that died, keeping those `scan` names, then the exits left
    /// beside no record, and returns what could not be swept.
    fn sweep_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure>;

    /// Sweeps the folder an earlier muster bound holder sockets in, once the
    /// holders `scan` finds serving there are adopted, and removes it when
    /// empty, returning what could not be swept.
    fn sweep_legacy_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure>;

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
