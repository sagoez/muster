use std::path::Path;

use crate::domain::{
    config::{ConfigError, WorkspaceConfig},
    project::Project,
};

/// Driven port: lists the registered projects and loads or writes workspace
/// files by path. A `config_path` may begin with `~`.
pub trait ProjectRegistry {
    /// The registered projects, in file order. A missing registry yields an
    /// empty list rather than an error.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the registry exists but cannot be read or parsed.
    fn projects(&self) -> Result<Vec<Project>, ConfigError>;

    /// Loads the workspace configuration at `config_path`.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the config cannot be read or parsed.
    fn workspace(&self, config_path: &Path) -> Result<WorkspaceConfig, ConfigError>;

    /// Whether a workspace config already exists at `config_path`.
    fn workspace_exists(&self, config_path: &Path) -> bool;

    /// Persists the full project list, replacing the registry file.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the registry cannot be written.
    fn save(&self, projects: &[Project]) -> Result<(), ConfigError>;

    /// Persists `config` to `config_path`, creating parent directories.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the workspace file cannot be written.
    fn save_workspace(
        &self,
        config_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<(), ConfigError>;

    /// Reads the workspace at `config_path`, applies `update`, and writes it
    /// back as a single operation, so concurrent updates to the same project
    /// (for example two `muster run` invocations) cannot lose each other. The
    /// default is an unlocked read-modify-write; a filesystem-backed adapter
    /// should override this to hold an exclusive lock across the whole sequence.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the workspace cannot be read or written.
    fn update_workspace(
        &self,
        config_path: &Path,
        update: &mut dyn FnMut(WorkspaceConfig) -> WorkspaceConfig,
    ) -> Result<(), ConfigError> {
        let config = self.workspace(config_path)?;
        self.save_workspace(config_path, &update(config))
    }
}
