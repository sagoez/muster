use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{agent::Agent, mcp_scope::McpScope};
use crate::domain::value::ProjectRoot;

/// The config an entry is added to or removed from: an agent's, of one
/// scope, for one project when the scope belongs to one.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct ConfigTarget {
    /// The agent.
    #[getset(get_copy = "pub")]
    agent: Agent,
    /// The scope.
    #[getset(get_copy = "pub")]
    scope: McpScope,
    /// The project a local or project scope belongs to.
    #[getset(get = "pub")]
    root: Option<ProjectRoot>,
}
