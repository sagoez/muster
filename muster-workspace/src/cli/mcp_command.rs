use clap::Subcommand;

use super::mcp_registration::McpRegistration;

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Register muster's MCP server with an agent.
    ///
    /// Runs the agent's own `mcp add` (`claude mcp add` or `codex mcp add`)
    /// in this folder, launching this muster binary, and exits as it does.
    /// Zed has no such command, so for zed this prints the entry to add to
    /// its settings and writes nothing.
    Add(McpRegistration),
    /// Unregister muster's MCP server from an agent.
    ///
    /// Runs the agent's own `mcp remove` in this folder and exits as it
    /// does. For zed this says what to delete from its settings.
    Remove(McpRegistration),
}
