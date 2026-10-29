use crate::domain::{
    config::{ConfigError, RecordedEdit},
    value::ProjectRoot,
};

/// Driven port: the agent edits each project's stack took, kept so the user
/// is told of them whether or not a service ran at the time. The
/// [`StackStore`](super::StackStore) write that applies an agent's edit
/// records it in the same transaction, and only the newest entries per
/// project are kept.
pub trait StackEditLog {
    /// The edits kept for the project at `root`, oldest first; none when it
    /// is not registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn edits(&self, root: &ProjectRoot) -> Result<Vec<RecordedEdit>, ConfigError>;
}
