use serde::Serialize;
use strum::{Display, EnumIter, EnumString};

/// Where in an agent's config an MCP server entry lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum McpScope {
    /// One project, kept in the user's own config and not shared.
    Local,
    /// One project, in a file inside it that can be committed.
    Project,
    /// Every project of the user.
    User,
}

impl McpScope {
    /// Whether an entry of this scope belongs to one project.
    pub fn needs_project(self) -> bool {
        matches!(self, Self::Local | Self::Project)
    }
}
