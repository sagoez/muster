use crate::domain::{
    config::{ConfigError, RecordedEdit, StackEdit},
    value::ProjectRoot,
};

/// Driven port: the agent edits each project's stack took, kept so the user
/// is told of them whether or not a service ran at the time. The store keeps
/// only the newest entries per project.
pub trait StackEditLog {
    /// Records `edit` to the stack of the project at `root`, stamped now.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] when `root` is not registered,
    /// or another [`ConfigError`] when the store cannot be written.
    fn record_edit(&self, root: &ProjectRoot, edit: &StackEdit) -> Result<(), ConfigError>;

    /// The edits kept for the project at `root`, oldest first; none when it
    /// is not registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn edits(&self, root: &ProjectRoot) -> Result<Vec<RecordedEdit>, ConfigError>;
}
