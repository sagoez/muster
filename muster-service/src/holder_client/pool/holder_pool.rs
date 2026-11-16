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
    /// The records of holders still serving, and of dead holders whose child
    /// still runs; every other record is deleted.
    fn live_records(&self) -> Vec<HolderRecord>;

    /// Every record read once, as [`Self::live_records`] keeps them, with
    /// the record files that cannot be read, which no supervisor can adopt.
    /// A service start reads this once and hands it to every sweep.
    fn scan_records(&self) -> RecordScan;

    /// Sweeps the sockets' folder, listing it once: deletes the handover
    /// files holders that never started left behind and the sockets of
    /// holders that died, keeping those `scan` names, and returns what could
    /// not be swept.
    fn sweep_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure>;

    /// Sweeps the folder an earlier muster bound holder sockets in, once the
    /// holders `scan` finds serving there are adopted, and removes it when
    /// empty, returning what could not be swept.
    fn sweep_legacy_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure>;

    /// The log every run of the process `name` of `project` writes to.
    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf;

    /// Removes the per-holder logs an earlier muster left behind.
    ///
    /// # Errors
    /// Returns the I/O error from removing them.
    fn remove_legacy_logs(&self) -> io::Result<()>;
}
