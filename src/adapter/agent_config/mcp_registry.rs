use super::{
    agent::Agent, agent_config_error::AgentConfigError, config_target::ConfigTarget,
    edit_plan::EditPlan, entry_change::EntryChange, server_name::McpServerName,
};

/// The agents' MCP configs, as the composition root hands them to the CLI:
/// which agents are installed or hold an entry, the edits that add muster's
/// server to their configs or remove it, and the writing of those edits.
pub trait McpRegistry: Sync {
    /// The agents whose config or binary is found, in a fixed order.
    fn present(&self) -> Vec<Agent>;

    /// The agents of `targets` whose config holds an entry under `name`.
    fn holding(&self, targets: &[ConfigTarget], name: &McpServerName) -> Vec<Agent>;

    /// The edits making `change` to the entry `name` in each of `targets`'
    /// configs, worked out against the files as they are now and not yet
    /// written, with the agents' own config locks held until the plan is
    /// committed or dropped. With `force`, another server's entry under
    /// `name`, or a turned-off one of muster's, is replaced or removed
    /// rather than refused or kept.
    ///
    /// # Errors
    /// Returns the [`AgentConfigError`] of a scope an agent lacks, a file
    /// muster cannot read or edit safely, an entry it must not touch, a
    /// lock that stays held, or `cancelled` saying so while waiting on one.
    fn plan(
        &self,
        targets: &[ConfigTarget],
        name: &McpServerName,
        change: EntryChange,
        force: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<EditPlan, AgentConfigError>;

    /// Writes every edit of `plan`, or none unless a write fails partway;
    /// `cancelled` saying so before the first write leaves every config as
    /// it was.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] for a file another writer
    /// changed since the plan, [`AgentConfigError::Interrupted`],
    /// [`AgentConfigError::Io`] for a file that cannot be written, or
    /// [`AgentConfigError::PartlyWritten`] naming the configs already
    /// written when a later one fails.
    fn commit(&self, plan: EditPlan, cancelled: &dyn Fn() -> bool) -> Result<(), AgentConfigError>;
}
