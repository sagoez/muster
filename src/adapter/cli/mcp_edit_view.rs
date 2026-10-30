use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::agent_config::{
    Agent, EditOutcome, EntryChange, McpScope, McpServerName, PlannedEdit,
};

/// What `muster mcp add` or `remove` did to each agent's config, as
/// `--json` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct McpEditView {
    /// Whether the entries were added or removed.
    #[getset(get_copy = "pub")]
    change: EntryChange,
    /// One per config, in the order the agents were given.
    #[getset(get = "pub")]
    edits: Vec<McpConfigEdit>,
}

/// What one agent's config now holds of muster's entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct McpConfigEdit {
    /// The agent.
    #[getset(get_copy = "pub")]
    agent: Agent,
    /// The config's scope.
    #[getset(get_copy = "pub")]
    scope: McpScope,
    /// The server's name in it.
    #[getset(get = "pub")]
    name: McpServerName,
    /// The file.
    #[getset(get = "pub")]
    path: PathBuf,
    /// What was done.
    #[getset(get_copy = "pub")]
    outcome: EditOutcome,
    /// What the agent does with the entry that a person should know.
    #[getset(get_copy = "pub")]
    note: Option<&'static str>,
}

impl McpConfigEdit {
    /// What `edit`, of the entry `name`, did.
    pub fn of(edit: &PlannedEdit, name: &McpServerName) -> Self {
        Self::builder()
            .agent(edit.target().agent())
            .scope(edit.target().scope())
            .name(name.clone())
            .path(edit.path().clone())
            .outcome(edit.outcome())
            .note(edit.note())
            .build()
    }
}
