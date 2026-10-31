use clap::Subcommand;

use super::mcp_entry_args::McpEntryArgs;

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Add muster's MCP server to agents' configs.
    ///
    /// Writes an entry launching this muster binary into Claude Code's,
    /// Codex's or Zed's own config, keeping everything else in the file.
    /// An identical entry is left alone and one an older muster wrote is
    /// updated; another server's under the name is replaced, and one Codex
    /// or Zed has turned off is turned back on, only with --force. muster
    /// does not read Claude Code's own turned-off list: if Claude Code has
    /// the server turned off, it is still reported added; turn it back on
    /// in Claude Code with /mcp.
    Add(McpEntryArgs),
    /// Remove muster's MCP server from agents' configs.
    ///
    /// Removes only the entry under the name, and only when it launches a
    /// muster binary unless --force is given.
    Remove(McpEntryArgs),
}
