use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    agent_config_error::AgentConfigError,
    config_file::io_error,
    config_lock::ConfigLock,
    file_edit::{FileEdit, WRITE},
    planned_edit::PlannedEdit,
};

/// Every edit of one command, worked out and not yet written, with the
/// locks of the configs whose agents lock them, held until it is committed
/// or dropped.
#[derive(Debug, Getters, TypedBuilder)]
pub struct EditPlan {
    /// One per config, in the order the agents were given.
    #[getset(get = "pub")]
    edits: Vec<PlannedEdit>,
    /// The agents' own locks, held from the check to the write.
    locks: Vec<ConfigLock>,
}

impl EditPlan {
    /// Writes every edit or none: each file is checked unchanged since it
    /// was read, each new text written and synced beside its file, each
    /// file checked again, and only then is every one put in place. A
    /// failure or `cancelled` before that leaves every config as it was
    /// and no temporary file behind.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] for a file another writer
    /// changed, [`AgentConfigError::Interrupted`] when `cancelled` says so,
    /// [`AgentConfigError::Io`] for a file that cannot be written, or
    /// [`AgentConfigError::PartlyWritten`] naming the configs already put
    /// in place when a later one fails.
    pub(super) fn commit(self, cancelled: &dyn Fn() -> bool) -> Result<(), AgentConfigError> {
        let Self { edits, locks } = self;
        let files: Vec<&FileEdit> = edits.iter().flat_map(PlannedEdit::files).collect();
        for file in &files {
            file.check_unchanged()?;
        }
        let mut staged = Vec::with_capacity(files.len());
        for file in &files {
            staged.push((*file, file.stage()?));
        }
        for file in &files {
            file.check_unchanged()?;
        }
        if cancelled() {
            return Err(AgentConfigError::Interrupted);
        }
        let mut written: Vec<PathBuf> = Vec::with_capacity(staged.len());
        for (file, staged) in staged {
            if let Err(source) = staged.commit() {
                let failure = io_error(WRITE, file.path(), source);
                return Err(if written.is_empty() {
                    failure
                } else {
                    AgentConfigError::PartlyWritten {
                        written,
                        source: Box::new(failure),
                    }
                });
            }
            written.push(file.path().clone());
        }
        drop(locks);
        Ok(())
    }
}
