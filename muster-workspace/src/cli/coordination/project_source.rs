use muster_domain::value::ProjectRoot;

use crate::cli::failure::error::CliError;

/// Where a coordination command finds its project, asked only once the
/// command's own input reads, so a write that registers its folder never
/// registers it for input it refuses.
pub trait ProjectSource {
    /// The root of the project the command acts on.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be found or
    /// registered.
    fn root(&self) -> Result<ProjectRoot, CliError>;
}

impl ProjectSource for ProjectRoot {
    /// This root, already found.
    fn root(&self) -> Result<ProjectRoot, CliError> {
        Ok(self.clone())
    }
}
