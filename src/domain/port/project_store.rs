use std::path::Path;

use crate::domain::{
    config::{ConfigError, UnreadableProject},
    project::Project,
    value::{EpochNanos, ProjectName, ProjectRoot},
};

/// Driven port: the registered projects. A project whose stored row does not
/// read is left out of every listing and lookup but its own, so one corrupt
/// row never breaks the others; [`ProjectStore::unreadable_projects`] names
/// it.
pub trait ProjectStore {
    /// Every registered project that reads, by name.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn projects(&self) -> Result<Vec<Project>, ConfigError>;

    /// Every registered project whose stored row does not read, by folder.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn unreadable_projects(&self) -> Result<Vec<UnreadableProject>, ConfigError>;

    /// The project registered at `root`, if any.
    ///
    /// # Errors
    /// Returns [`ConfigError::CorruptRow`] naming `root` when its row does
    /// not read, or another [`ConfigError`] when the store cannot be read.
    fn project_at(&self, root: &ProjectRoot) -> Result<Option<Project>, ConfigError>;

    /// The project registered as `name`, if any.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn project_named(&self, name: &ProjectName) -> Result<Option<Project>, ConfigError>;

    /// When the project at `root` was registered, if it is.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read or holds a
    /// time before the epoch.
    fn registered_at(&self, root: &ProjectRoot) -> Result<Option<EpochNanos>, ConfigError>;

    /// The registered project whose root is `dir` or its nearest ancestor,
    /// among those that read.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn project_containing(&self, dir: &Path) -> Result<Option<Project>, ConfigError>;

    /// Deletes the project registered at `root`, readable or not, with its
    /// processes and coordination. Returns whether it existed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn remove_project(&self, root: &ProjectRoot) -> Result<bool, ConfigError>;

    /// Deletes the project stored at exactly `stored`, as
    /// [`ProjectStore::unreadable_projects`] lists it, with its processes and
    /// coordination, only while its row still does not read; that covers a
    /// root no [`ProjectRoot`] holds. Returns whether it was deleted.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn remove_unreadable_project(&self, stored: &Path) -> Result<bool, ConfigError>;
}
