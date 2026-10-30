use super::StackReader;
use crate::domain::{
    config::{
        AppliedChange, ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessSpec,
        Registration,
    },
    coordination::Author,
    project::Project,
    value::{IdleStop, ProcessName, ProjectRoot},
};

/// Driven port: each project's process definitions, read and written. Every
/// write is one atomic transaction, and moves the project's
/// [`StackRevision`](crate::domain::value::StackRevision). An edit `author`
/// makes that [`StackEdit::is_logged`](crate::domain::config::StackEdit::is_logged)
/// keeps is recorded in the project's [`StackEditLog`](super::StackEditLog)
/// in that same transaction, so the edit and its entry never exist apart.
pub trait StackStore: StackReader {
    /// Adds `spec` for `author`, registering `root` first when it is not a
    /// project.
    ///
    /// # Errors
    /// Returns [`ConfigError::DuplicateProcessName`] when the name is taken, or
    /// another [`ConfigError`] when the store cannot be written.
    fn add_process(
        &self,
        root: &ProjectRoot,
        spec: &ProcessSpec,
        author: &Author,
    ) -> Result<Registration, ConfigError>;

    /// Applies `change` to the process `name` for `author` and returns the
    /// result, and whether it changed anything; one that changed nothing
    /// writes nothing.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] or [`ConfigError::UnknownProcess`]
    /// when there is nothing to change, or another [`ConfigError`] when the
    /// store cannot be written.
    fn change_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
        author: &Author,
    ) -> Result<AppliedChange, ConfigError>;

    /// Removes the process `name` for `author` and returns what it was.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] or [`ConfigError::UnknownProcess`]
    /// when there is nothing to remove, or another [`ConfigError`] when the
    /// store cannot be written.
    fn remove_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        author: &Author,
    ) -> Result<ProcessSpec, ConfigError>;

    /// Plans and applies an import in one transaction, registering `root` when new.
    ///
    /// # Errors
    /// Returns what [`Stack::plan_import`](crate::domain::config::Stack::plan_import) refuses, or another [`ConfigError`]
    /// when the store cannot be written; nothing is written on an error.
    fn import(
        &self,
        root: &ProjectRoot,
        processes: &[ProcessSpec],
        mode: ImportMode,
    ) -> Result<(Registration, ImportPlan), ConfigError>;

    /// Sets or clears (`None`) the project's idle-stop override, bumping its
    /// stack revision, and returns the project as stored.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] when `root` is not registered,
    /// or another [`ConfigError`] when the store cannot be written.
    fn set_idle_stop(
        &self,
        root: &ProjectRoot,
        idle_stop: Option<IdleStop>,
    ) -> Result<Project, ConfigError>;
}
