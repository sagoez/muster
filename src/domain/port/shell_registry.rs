use std::path::PathBuf;

use crate::domain::{
    occupancy::{ReportStamp, ShellId, ShellRecord, ShellRegistryError},
    value::Pid,
};

/// Driven port: the live shells running muster's hook, the last report each
/// one made, and the notes left for each to print at its next prompt.
pub trait ShellRegistry: Send + Sync {
    /// The shell running as `pid` now, with its start token.
    fn identify(&self, pid: Pid) -> Option<ShellId>;

    /// Whether `shell` still runs: its pid is alive with the same start token.
    fn is_live(&self, shell: &ShellId) -> bool;

    /// Every record that reads; unreadable files are skipped.
    fn records(&self) -> Vec<ShellRecord>;

    /// The record of `shell`, none when absent or written by another process
    /// on that pid.
    fn record(&self, shell: &ShellId) -> Option<ShellRecord>;

    /// Writes `record` unless the stored one of the same shell is as new or
    /// newer. Returns whether it wrote.
    ///
    /// # Errors
    /// Returns [`ShellRegistryError::Write`] when the record cannot be written.
    fn save_if_newer(&self, record: &ShellRecord) -> Result<bool, ShellRegistryError>;

    /// Rewrites the stored record of `record`'s shell as `record`, only
    /// while it is still the report of the same stamp and this user's own.
    /// Returns whether it wrote.
    ///
    /// # Errors
    /// Returns [`ShellRegistryError::Write`] when the record cannot be written.
    fn amend(&self, record: &ShellRecord) -> Result<bool, ShellRegistryError>;

    /// Leaves `lines`, answered to the report stamped `stamp`, for the
    /// shell's hook to print at its next prompt.
    ///
    /// # Errors
    /// Returns [`ShellRegistryError::Write`] when the note cannot be written.
    fn deliver(
        &self,
        shell: &ShellId,
        stamp: ReportStamp,
        lines: &[String],
    ) -> Result<(), ShellRegistryError>;

    /// Removes the shell's record and undelivered notes.
    fn forget(&self, shell: &ShellId);

    /// The folder the hook reads notes from.
    fn notes_dir(&self) -> PathBuf;
}
