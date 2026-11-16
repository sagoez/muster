use clap::Subcommand;

use super::mcp_registration::McpRegistration;

/// The subcommand of `muster mcp` that prints a session's context.
pub const CONTEXT_SUBCOMMAND: &str = "context";

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Register muster's MCP server with an agent.
    ///
    /// Runs the agent's own CLI in this folder, launching this muster
    /// binary. For Claude Code it replaces muster's entry in the scope with
    /// one that loads muster's tools up front, then adds a SessionStart hook
    /// running `muster mcp context` to that scope's settings file
    /// (~/.claude/settings.json for user, .claude/settings.json for project,
    /// .claude/settings.local.json for local). For Codex it adds the same
    /// hook to ~/.codex/hooks.json, which Codex runs once you trust it in
    /// /hooks. Zed has no such command or hook, so for zed this prints the
    /// entry to add to its settings and writes nothing.
    Add(McpRegistration),
    /// Unregister muster's MCP server from an agent.
    ///
    /// Takes muster's SessionStart hook out of the agent's hooks file,
    /// leaving every other hook, then runs the agent's own `mcp remove` in
    /// this folder and exits as it does. Claude Code without --scope loses
    /// the hook from every scope's file. For zed this says what to delete
    /// from its settings.
    Remove(McpRegistration),
    /// Print this project's notes and open todos for an agent's session.
    ///
    /// The SessionStart hook `muster mcp add` installs runs this, so each
    /// session, and each one resumed, cleared or compacted, starts knowing
    /// what other agents left. It prints at most a few thousand characters,
    /// and nothing outside a project or on any failure, always exiting 0 so
    /// the hook never fails a session.
    #[command(name = CONTEXT_SUBCOMMAND)]
    Context,
}
