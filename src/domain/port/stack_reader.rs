use crate::domain::{
    config::{ConfigError, Stack},
    value::{ProjectRoot, StackRevision},
};

/// Driven port: each project's process definitions, read only, for the
/// service, which never writes the stack.
pub trait StackReader {
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
}
