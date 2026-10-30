use std::path::PathBuf;

use super::{error::HolderError, record::HolderRecord};

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
}

impl Drop for RecordGuard {
    /// Removes the record unless [`RecordGuard::remove`] already did.
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.record.remove(&self.dir);
        }
    }
}
