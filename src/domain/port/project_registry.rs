use std::path::Path;

use crate::domain::{
    config::{ConfigError, WorkspaceConfig},
    project::Project,
};

/// Driven port: lists the registered projects and loads or writes workspace
/// files by path. A registered `config_path` is absolute or begins with `~`.
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

    /// Reads the workspace at `config_path` and runs `read` against it while the
    /// exclusive workspace lock is held, so a caller can coordinate an external
    /// action - retiring a deleted agent's durable session, say - with concurrent
    /// config writes: another instance writing the same `muster.yml` is blocked
    /// until `read` returns, so no re-add can land between the config check and the
    /// action. The default is an unlocked load; a filesystem-backed adapter should
    /// override it to hold the same lock `update_workspace` uses.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the workspace cannot be read or locked, or `read`
    /// itself fails.
    fn with_workspace_locked(
        &self,
        config_path: &Path,
        read: &mut dyn FnMut(&WorkspaceConfig) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        let config = self.workspace(config_path)?;
        read(&config)
    }

    /// Creates the workspace at `config_path` only when nothing exists there.
    /// Returns whether this call created the file. The default checks then
    /// writes; a filesystem-backed adapter should override it with an atomic
    /// exclusive create so a concurrent creation is never overwritten.
    ///
    /// # Errors
    /// Returns a `ConfigError` when the workspace cannot be written.
    fn create_workspace(
        &self,
        config_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<bool, ConfigError> {
        if self.workspace_exists(config_path) {
            return Ok(false);
        }
        self.save_workspace(config_path, config)?;
        Ok(true)
    }

    /// Applies `update` to the project list under the registry's exclusive lock
    /// where the implementation provides one, so concurrent mutations serialize
    /// instead of losing writes.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the registry cannot be read, locked, or written.
    fn update_projects(
        &self,
        update: &mut dyn FnMut(Vec<Project>) -> Vec<Project>,
    ) -> Result<(), ConfigError> {
        let projects = self.projects()?;
        self.save(&update(projects))
    }
}
