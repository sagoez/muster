use clap::Subcommand;

use super::mcp_entry_args::McpEntryArgs;

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Add muster's MCP server to agents' configs.
    ///
    /// Writes an entry launching this muster binary into Claude Code's,
    /// Codex's or Zed's own config, keeping everything else in the file.
    /// An identical entry is left alone; a different one under the name is
    /// replaced only with --force.
    Add(McpEntryArgs),
    /// Remove muster's MCP server from agents' configs.
    ///
    /// Removes only the entry under the name, and only when it launches
    /// this muster unless --force is given.
    Remove(McpEntryArgs),
}
