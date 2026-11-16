use muster_domain::{
    config::{ConfigError, WritePlace, Written},
    coordination::Author,
    value::ProjectRoot,
};

use crate::cli::failure::error::CliError;

/// Where a coordination command finds its project. A read, a delete or a
/// status change asks for a registered project; a shared write, such as
/// `note set`, goes through [`ProjectSource::share`], the one way a command
/// registers its project.
pub trait ProjectSource {
    /// The root of the registered project the command acts on.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be found.
    fn root(&self) -> Result<ProjectRoot, CliError>;

    /// Runs `write`, a shared write by `by`, at the place it goes, which it
    /// registers in its own transaction when it needs a project.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a place that cannot be found or
    /// registered, or the write's own.
    fn share<T>(
        &self,
        by: &Author,
        write: impl FnOnce(&WritePlace) -> Result<Written<T>, ConfigError>,
    ) -> Result<Written<T>, CliError>;
}

#[cfg(test)]
impl ProjectSource for ProjectRoot {
    /// This root, already found.
    fn root(&self) -> Result<ProjectRoot, CliError> {
        Ok(self.clone())
    }

    /// Writes to this registered project.
    fn share<T>(
        &self,
        _by: &Author,
        write: impl FnOnce(&WritePlace) -> Result<Written<T>, ConfigError>,
    ) -> Result<Written<T>, CliError> {
        Ok(write(&WritePlace::from(self.clone()))?)
    }
}
