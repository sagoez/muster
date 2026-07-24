use std::path::PathBuf;

use crate::domain::{
    agent_session::{
        AgentProcessId, AgentProcessStartToken, AgentSession, AgentSessionId, AgentSessionState,
        NativeSessionId,
    },
    config::ConfigError,
    process::AgentTool,
};

/// Persists agent-session identity and history across TUI lifetimes.
pub trait AgentSessionStore {
    /// Loads sessions in history order, oldest first.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be read or parsed.
    fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError>;

    /// Returns the durable state-file location inherited by provider hooks.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state location cannot be resolved.
    fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError>;

    /// Inserts or replaces a session and makes it the newest history entry.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be updated.
    fn upsert(&self, session: &AgentSession) -> Result<(), ConfigError>;

    /// Changes a session's open/closed state and moves it to the end of history.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be updated.
    fn set_state(&self, id: &AgentSessionId, state: AgentSessionState) -> Result<(), ConfigError>;

    /// Binds a session to the process currently launched on its behalf.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session cannot be updated.
    fn set_owner_process_id(
        &self,
        id: &AgentSessionId,
        process_id: AgentProcessId,
        process_start_token: Option<AgentProcessStartToken>,
        wrapper_process_id: Option<AgentProcessId>,
    ) -> Result<(), ConfigError>;

    /// Records the latest identity reported by the session's owning provider.
    /// Same-provider conversation changes replace the previous identity.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be updated or the
    /// lifecycle event comes from a different provider.
    fn capture_native_id(
        &self,
        id: &AgentSessionId,
        provider: AgentTool,
        process_id: AgentProcessId,
        parent_process_id: Option<AgentProcessId>,
        native_id: NativeSessionId,
    ) -> Result<(), ConfigError>;
}
