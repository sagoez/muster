use std::{path::Path, sync::Arc};

use super::mcp_launch::McpLaunch;
use crate::{
    adapter::{client::ServiceAccess, no_state_dir::NoStateDir},
    domain::{
        config::ConfigError,
        port::{LocalState, ShellRegistry, StateChangeNotifier},
    },
    error::MusterError,
};

/// What the composition root hands the CLI: muster's state and service as
/// ports, and how the MCP server and the hidden service and holder entry
/// points run, so the CLI builds none of them itself.
pub trait CliPorts: Sync {
    /// Opens muster's local state, creating it when absent, for the commands
    /// that read or write it.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the database cannot be opened.
    fn open_state(&self) -> Result<Arc<dyn LocalState>, ConfigError>;

    /// Opens muster's local state read-only when it already exists at the
    /// current schema, creating and migrating nothing.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the database cannot be read.
    fn existing_state(&self) -> Result<Option<Arc<dyn LocalState>>, ConfigError>;

    /// How the CLI reaches this user's service.
    fn service(&self) -> Arc<dyn ServiceAccess>;

    /// What tells a running service of each change the CLI stores.
    fn change_notifier(&self) -> Arc<dyn StateChangeNotifier>;

    /// The shells running muster's hook: their records and notes.
    ///
    /// # Errors
    /// Returns [`NoStateDir`] when the platform offers no state directory to
    /// keep them in.
    fn shell_registry(&self) -> Result<Arc<dyn ShellRegistry>, NoStateDir>;

    /// Whether muster's state directory exists and this process's effective
    /// user owns it.
    fn owns_state_dir(&self) -> bool;

    /// Serves muster over MCP on stdio as `launch` describes, until the
    /// connecting agent disconnects.
    ///
    /// # Errors
    /// Returns a [`MusterError`] when the server cannot start or fails.
    fn serve_mcp(&self, launch: McpLaunch) -> Result<(), MusterError>;

    /// The config snippet an agent pastes in to run `executable` as its MCP
    /// server.
    fn mcp_config(&self, executable: &Path) -> String;

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
