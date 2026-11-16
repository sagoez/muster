use std::path::PathBuf;

use muster_domain::pty::ExitOutcome;
use muster_protocol::holder::{HolderError, HolderRecord};

/// Removes the holder record on every exit path, so a failed holder never
/// leaves a record pointing at a socket nobody serves.
pub(super) struct RecordGuard {
    pub(super) record: HolderRecord,
    pub(super) dir: PathBuf,
    pub(super) removed: bool,
}

impl RecordGuard {
    /// Removes the record now, reporting a failure the drop would swallow.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the record file cannot be removed.
    pub(super) fn remove(mut self) -> Result<(), HolderError> {
        self.removed = true;
        Ok(self.record.remove(&self.dir)?)
    }

    /// Keeps the record and leaves `outcome` beside it, for the next service
    /// to report: the end of a holder that could not be reached to have its
    /// child's exit collected.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the exit cannot be written; the record
    /// is then removed as on any other failure.
    pub(super) fn strand(mut self, outcome: &ExitOutcome) -> Result<(), HolderError> {
        self.record.save_exit(&self.dir, outcome)?;
        self.removed = true;
        Ok(())
    }
}

impl Drop for RecordGuard {
    /// Removes the record unless [`RecordGuard::remove`] already did.
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.record.remove(&self.dir);
        }
    }
}
