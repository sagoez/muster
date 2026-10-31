use getset::{CopyGetters, Getters};

use super::args::{agent_parser, mcp_scope_parser, server_name};
use crate::adapter::agent_config::{Agent, DEFAULT_SERVER_NAME, McpScope, McpServerName};

/// Separates several agents in one `--client`.
const CLIENT_SEPARATOR: char = ',';
/// The id of `--force`, whose help `muster mcp remove` words for itself.
pub(super) const FORCE_ARG: &str = "force";

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
    /// Which config: user (every project, the default), local (this
    /// project, in your own Claude Code config) or project (a file in this
    /// project, .mcp.json or .codex/config.toml, shared with whoever
    /// clones it). Zed has only user.
    #[arg(long, value_parser = mcp_scope_parser())]
    #[getset(get_copy = "pub")]
    scope: Option<McpScope>,
    /// The server's name in the agent's config.
    #[arg(long, default_value = DEFAULT_SERVER_NAME, value_parser = server_name)]
    #[getset(get = "pub")]
    name: McpServerName,
    /// Replace a server under the name that runs something else, or turn
    /// muster's own back on in Codex or Zed.
    #[arg(id = FORCE_ARG, long)]
    #[getset(get_copy = "pub")]
    force: bool,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
}
