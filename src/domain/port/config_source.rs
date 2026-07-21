use crate::domain::config::{ConfigError, WorkspaceConfig};

/// Driven port: a source that can load the workspace configuration.
pub trait ConfigSource {
    /// Loads and validates the workspace configuration.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the source cannot be read or is invalid.
    fn load(&self) -> Result<WorkspaceConfig, ConfigError>;
}
