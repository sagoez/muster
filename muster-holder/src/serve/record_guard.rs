use std::path::PathBuf;

use muster_protocol::holder::{HolderError, HolderRecord, LeftExit};

/// Removes the holder record on every exit path but one, so a failed holder
/// never leaves a record pointing at a socket nobody serves: a holder that
/// strands its child's exit keeps the record on purpose, with the exit
/// beside it, for the next service to report.
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

    /// Keeps the record, saving it again should something have removed it,
    /// and leaves `left` beside it, for the next service to report: the end
    /// of a holder that could not be reached to have its child's exit
    /// collected.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the record or the exit cannot be
    /// written; the record is then removed as on any other failure.
    pub(super) fn strand(mut self, left: &LeftExit) -> Result<(), HolderError> {
        self.record.save_stranded(&self.dir, left)?;
        self.removed = true;
        Ok(())
    }
}

impl Drop for RecordGuard {
    /// Removes the record unless [`RecordGuard::remove`] or
    /// [`RecordGuard::strand`] already settled it.
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.record.remove(&self.dir);
        }
    }
}
