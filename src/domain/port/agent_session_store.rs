use std::path::PathBuf;

use crate::domain::{
    agent_session::{
        AgentProcessId, AgentProcessStartToken, AgentSession, AgentSessionId, AgentSessionState,
        LaunchToken, NativeSessionId,
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

    /// Binds a session to the process currently launched on its behalf. When
    /// `launch_token` is `Some`, the token muster injected into the agent's
    /// environment is recorded as part of the same claim, so it is set only if the
    /// claim succeeds: a rejected competing launch cannot invalidate the owning
    /// launch's token. The owner-refreshing launcher claim passes `None`.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session cannot be updated or a live owner
    /// already holds it.
    fn set_owner_process_id(
        &self,
        id: &AgentSessionId,
        process_id: AgentProcessId,
        process_start_token: Option<AgentProcessStartToken>,
        launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError>;

    /// Records the identity a session's provider reported. The report is trusted by
    /// session id: muster injects the id into the launched process's environment,
    /// so only that process tree can produce it, whatever launcher layers sit
    /// between. `reporter_process_id` binds the identity to the first reporting
    /// process so a nested same-provider launch cannot overwrite it, and
    /// `reported_launch_token` (the token from the reporting process's environment)
    /// must match the current launch so a stale capture from a previous launch is
    /// ignored.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be updated or the
    /// lifecycle event comes from a different provider.
    fn capture_native_id(
        &self,
        id: &AgentSessionId,
        provider: AgentTool,
        native_id: NativeSessionId,
        reporter_process_id: AgentProcessId,
        reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError>;
}
