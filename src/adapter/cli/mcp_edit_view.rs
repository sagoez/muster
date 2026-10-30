use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::mcp_config_edit::McpConfigEdit;
use crate::adapter::agent_config::EntryChange;

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
