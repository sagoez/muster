use getset::{CopyGetters, Getters};

use super::args::{agent_parser, mcp_scope_parser, server_name};
use crate::adapter::agent_config::{Agent, DEFAULT_SERVER_NAME, McpScope, McpServerName};

/// Separates several agents in one `--client`.
const CLIENT_SEPARATOR: char = ',';

/// Which agents' configs `muster mcp add` or `remove` changes, and where.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct McpEntryArgs {
    /// The agent: claude (Claude Code), codex or zed. Repeat it or separate
    /// with commas for several. At a terminal, chosen from a list of those
    /// installed when absent; required elsewhere.
    #[arg(
        long,
        value_name = "AGENT",
        value_parser = agent_parser(),
        value_delimiter = CLIENT_SEPARATOR
    )]
    #[getset(get = "pub")]
    client: Vec<Agent>,
    /// Which config: local (this project, in your own Claude Code config),
    /// project (a file in this project, .mcp.json or .codex/config.toml) or
    /// user (every project). Defaults to local for Claude Code and user for
    /// Codex and Zed; Zed has only user.
    #[arg(long, value_parser = mcp_scope_parser())]
    #[getset(get_copy = "pub")]
    scope: Option<McpScope>,
    /// The server's name in the agent's config.
    #[arg(long, default_value = DEFAULT_SERVER_NAME, value_parser = server_name)]
    #[getset(get = "pub")]
    name: McpServerName,
    /// Replace or remove a server under the name that runs something else.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    force: bool,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
}
