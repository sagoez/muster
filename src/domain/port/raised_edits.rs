use crate::domain::{
    config::ConfigError,
    value::{EditId, ProjectRoot},
};

/// Driven port: the newest agent edit of each project a service raised as an
/// event, so a later service raises only what no service did, and never wakes
/// an agent to an edit it could already have seen.
pub trait RaisedEdits: Send + Sync {
    /// The newest edit of the project at `root` already raised; none before
    /// any was, or when it is not registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn raised_through(&self, root: &ProjectRoot) -> Result<Option<EditId>, ConfigError>;

    /// Records that every edit of the project at `root` up to `edit` was
    /// raised. A project no longer registered is skipped.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn mark_raised(&self, root: &ProjectRoot, edit: EditId) -> Result<(), ConfigError>;
}
