use crate::domain::{
    config::{
        ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessSpec, Registration, Stack,
    },
    value::{ProcessName, ProjectRoot, StackRevision},
};

/// Driven port: each project's process definitions. Every write is one atomic
/// transaction, and moves the project's [`StackRevision`].
pub trait StackStore {
    /// The project's stack, or none when `root` is not registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError>;

    /// The revision the project's stack is at, or none when `root` is not
    /// registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn stack_revision(&self, root: &ProjectRoot) -> Result<Option<StackRevision>, ConfigError>;

    /// Adds `spec`, registering `root` first when it is not a project.
    ///
    /// # Errors
    /// Returns [`ConfigError::DuplicateProcessName`] when the name is taken, or
    /// another [`ConfigError`] when the store cannot be written.
    fn add_process(
        &self,
        root: &ProjectRoot,
        spec: &ProcessSpec,
    ) -> Result<Registration, ConfigError>;

    /// Applies `change` to the process `name` and returns the result.
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
    ) -> Result<ProcessSpec, ConfigError>;

    /// Removes the process `name` and returns what it was.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] or [`ConfigError::UnknownProcess`]
    /// when there is nothing to remove, or another [`ConfigError`] when the
    /// store cannot be written.
    fn remove_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
    ) -> Result<ProcessSpec, ConfigError>;

    /// Plans and applies an import in one transaction, registering `root` when new.
    ///
    /// # Errors
    /// Returns what [`Stack::plan_import`] refuses, or another [`ConfigError`]
    /// when the store cannot be written; nothing is written on an error.
    fn import(
        &self,
        root: &ProjectRoot,
        processes: &[ProcessSpec],
        mode: ImportMode,
    ) -> Result<(Registration, ImportPlan), ConfigError>;
}
