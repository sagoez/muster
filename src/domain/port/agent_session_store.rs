use std::path::{Path, PathBuf};

use crate::domain::{
    agent_session::{
        AgentProcessId, AgentProcessStartToken, AgentSession, AgentSessionId, AgentSessionState,
        ConfiguredAgentKey, LaunchToken, NativeSessionId,
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

    /// Atomically binds a configured agent to its durable session under the store
    /// lock, returning the resulting session id. If a session matching
    /// `candidate`'s project and configured key already exists, its provider and
    /// launch command are refreshed on that latest record (preserving any
    /// concurrently captured native id, owner, or lifecycle state); otherwise
    /// `candidate` is inserted. The read and write happen in one transaction so
    /// two instances first opening a project cannot create duplicate records.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be read or written.
    fn link_configured(&self, candidate: &AgentSession) -> Result<AgentSessionId, ConfigError>;

    /// Adopts a conversation as a configured agent's durable session in one
    /// transaction. `session` carries the source conversation's id and native
    /// identity, now stamped with the configured project and key. Any record with
    /// the same id (the source history record it was built from) and any prior
    /// record for the same project and configured key are removed before it is
    /// inserted as the newest entry, so the pinned identity has exactly one
    /// launchable owner and reconciliation cannot resume a stale record instead.
    /// The removed records are returned so a caller can restore the complete
    /// pre-pin state if a following step (the config write) fails.
    ///
    /// The source is validated under the lock to still be unconfigured, or already
    /// this exact target; a source a concurrent pin already claimed for a different
    /// agent is rejected rather than clobbered.
    ///
    /// # Errors
    /// Returns [`ConfigError::AgentSessionAlreadyPinned`] when the source was
    /// concurrently pinned elsewhere, or a [`ConfigError`] when the state file
    /// cannot be read or written.
    fn pin_configured(&self, session: &AgentSession) -> Result<Vec<AgentSession>, ConfigError>;

    /// Undoes a [`AgentSessionStore::pin_configured`] whose follow-up failed,
    /// restoring each displaced record so nothing it removed is lost. `expected` is
    /// the record the pin transferred: the displaced record with its id is restored
    /// only while the current record still belongs to that pin, so a concurrent
    /// re-target is not clobbered. Each restored record keeps its original identity
    /// but adopts the runtime state (captured id, owner, lifecycle) read from the
    /// current same-id record under the lock, so a capture made during the failed
    /// write is not discarded.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be read or written.
    fn restore_sessions(
        &self,
        expected: &AgentSession,
        records: &[AgentSession],
    ) -> Result<(), ConfigError>;

    /// Un-configures durable sessions under `project` whose configured key is not in
    /// `live_keys` - configured records left without a matching `muster.yml` agent,
    /// for example a pin whose config write never completed before a crash. Clearing
    /// the key returns them to history so they are restored and offered for pinning
    /// again instead of being stranded, hidden from both pickers. Returns the number
    /// of records changed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the state file cannot be read or written.
    fn retire_orphaned_configured(
        &self,
        project: &Path,
        live_keys: &[ConfiguredAgentKey],
    ) -> Result<usize, ConfigError>;

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
