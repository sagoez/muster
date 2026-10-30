use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    agent_config_error::AgentConfigError,
    config_file::io_error,
    config_lock::ConfigLock,
    planned_edit::{PlannedEdit, WRITE},
};

/// Every edit of one command, worked out and not yet written, with the
/// locks of the configs whose agents lock them, held until it is committed
/// or dropped.
#[derive(Debug, Getters, TypedBuilder)]
pub struct EditPlan {
    /// One per config, in the order the agents were given.
    #[getset(get = "pub")]
    edits: Vec<PlannedEdit>,
    /// The agents' own locks, held from the read to the write.
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
        let writing: Vec<&PlannedEdit> = edits.iter().filter(|edit| edit.writes()).collect();
        for edit in &writing {
            edit.check_unchanged()?;
        }
        let mut staged = Vec::with_capacity(writing.len());
        for edit in &writing {
            if let Some(file) = edit.stage()? {
                staged.push((*edit, file));
            }
        }
        for edit in &writing {
            edit.check_unchanged()?;
        }
        if cancelled() {
            return Err(AgentConfigError::Interrupted);
        }
        let mut written: Vec<PathBuf> = Vec::with_capacity(staged.len());
        for (edit, file) in staged {
            if let Err(source) = file.commit() {
                let failure = io_error(WRITE, edit.path(), source);
                return Err(if written.is_empty() {
                    failure
                } else {
                    AgentConfigError::PartlyWritten {
                        written,
                        source: Box::new(failure),
                    }
                });
            }
            written.push(edit.path().clone());
        }
        drop(locks);
        Ok(())
    }
}
