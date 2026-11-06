use strum::{Display, EnumIter, EnumString};

/// Where Claude Code keeps a server `muster mcp add` registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum McpScope {
    /// This folder's project, in the user's own Claude Code config.
    Local,
    /// This folder's `.mcp.json`, shared with whoever clones it.
    Project,
    /// Every project of the user.
    #[default]
    User,
}
