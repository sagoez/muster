use getset::CopyGetters;

use crate::cli::{command_line::args::mcp_agent_parser, mcp::mcp_agent::McpAgent};

/// The agent `muster mcp add` or `remove` asks.
#[derive(clap::Args, CopyGetters)]
pub struct McpRegistration {
    /// The agent: claude (Claude Code), codex or zed (the Zed editor).
    #[arg(value_parser = mcp_agent_parser())]
    #[getset(get_copy = "pub")]
    agent: McpAgent,
}
