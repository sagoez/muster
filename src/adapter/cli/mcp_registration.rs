use getset::CopyGetters;

use super::{
    args::{mcp_agent_parser, mcp_scope_parser},
    mcp_agent::McpAgent,
    mcp_scope::McpScope,
};

/// The agent `muster mcp add` or `remove` asks, and the scope.
#[derive(clap::Args, CopyGetters)]
pub struct McpRegistration {
    /// The agent: claude (Claude Code), codex or zed (the Zed editor).
    #[arg(value_parser = mcp_agent_parser())]
    #[getset(get_copy = "pub")]
    agent: McpAgent,
    /// Claude Code's scope: user (every project, the default), local (this
    /// folder's project, in your own config) or project (this folder's
    /// .mcp.json). Codex and Zed take servers for the user only.
    #[arg(long, value_parser = mcp_scope_parser())]
    #[getset(get_copy = "pub")]
    scope: Option<McpScope>,
}
