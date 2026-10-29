use std::path::Path;

use crate::domain::{
    config::ConfigError,
    project::Project,
    value::{ProjectName, ProjectRoot},
};

/// Driven port: the registered projects.
pub trait ProjectStore {
    /// Every registered project, by name.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn projects(&self) -> Result<Vec<Project>, ConfigError>;

    /// The project registered at `root`, if any.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn project_at(&self, root: &ProjectRoot) -> Result<Option<Project>, ConfigError>;

    /// The project registered as `name`, if any.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn project_named(&self, name: &ProjectName) -> Result<Option<Project>, ConfigError>;

    /// The registered project whose root is `dir` or its nearest ancestor.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn project_containing(&self, dir: &Path) -> Result<Option<Project>, ConfigError>;

    /// Deletes the project with its processes and coordination. Returns whether it existed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn remove_project(&self, root: &ProjectRoot) -> Result<bool, ConfigError>;
}
