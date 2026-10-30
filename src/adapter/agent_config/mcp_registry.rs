use super::{
    agent::Agent, agent_config_error::AgentConfigError, config_target::ConfigTarget,
    entry_change::EntryChange, planned_edit::PlannedEdit, server_name::McpServerName,
};

/// The agents' MCP configs, as the composition root hands them to the CLI:
/// which agents are installed, and the edit that adds muster's server to
/// one config or removes it.
pub trait McpRegistry: Sync {
    /// The agents whose config or binary is found, in a fixed order.
    fn present(&self) -> Vec<Agent>;

    /// The edit making `change` to the entry `name` in `target`'s config,
    /// worked out against the file as it is now and not yet written. With
    /// `force`, a different entry under `name` is replaced or removed rather
    /// than refused.
    ///
    /// # Errors
    /// Returns the [`AgentConfigError`] of a scope the agent lacks, a file
    /// muster cannot read or edit safely, or an entry it must not touch.
    fn plan(
        &self,
        target: &ConfigTarget,
        name: &McpServerName,
        change: EntryChange,
        force: bool,
    ) -> Result<PlannedEdit, AgentConfigError>;
}
