use std::path::Path;

use crate::{adapter::store::SqliteStore, domain::config::ConfigError, error::MusterError};

/// What the composition root hands the CLI: where muster's state lives and
/// how the hidden service and holder entry points run.
pub trait CliPorts {
    /// Opens muster's local state, for the commands that read or write it.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the database cannot be opened.
    fn open_state(&self) -> Result<SqliteStore, ConfigError>;

    /// Runs this user's service until a client stops or replaces it.
    ///
    /// # Errors
    /// Returns a [`MusterError`] when the service cannot start.
    fn run_service(&self) -> Result<(), MusterError>;

    /// Runs one process holder from the spec file at `spec`.
    ///
    /// # Errors
    /// Returns a [`MusterError`] when the holder cannot start.
    fn run_holder(&self, spec: &Path) -> Result<(), MusterError>;
}
