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
    /// binary. For Claude Code the entry loads muster's tools up front, and
    /// a SessionStart hook running `muster mcp context` goes in that scope's
    /// settings file (~/.claude/settings.json for user, .claude/settings.json
    /// for project, .claude/settings.local.json for local). For Codex the
    /// same hook goes in ~/.codex/hooks.json, which Codex runs once you
    /// trust it in /hooks, and an entry Codex already has is pointed at this
    /// binary in its config.toml. Adding again keeps an entry's arguments
    /// and environment. Only the scope's own files are read, checked first
    /// and edited in place; Ctrl-C waits while they change, and a step that
    /// fails puts back what the others changed. Zed has no such command or
    /// hook, so for zed this prints the entry to add to its settings and
    /// writes nothing.
    Add(McpRegistration),
    /// Unregister muster's MCP server from an agent.
    ///
    /// Runs the agent's own `mcp remove` in this folder, and once it
    /// succeeds takes muster's SessionStart hook out of the hooks file of
    /// the scope it removed muster from, leaving every other hook. A remove
    /// that fails changes nothing and exits as the agent did. For zed this
    /// says what to delete from its settings.
    Remove(McpRegistration),
    /// Print this project's notes and open todos for an agent's session.
    ///
    /// The SessionStart hook `muster mcp add` installs runs this, so each
    /// session, and each one resumed, cleared or compacted, starts knowing
    /// what other agents left, fenced and labelled as information, never
    /// instructions. It prints at most a few thousand bytes, the newest
    /// notes first, once per session start the hook input names, and
    /// nothing outside a project or on any failure, always exiting 0 so the
    /// hook never fails a session.
    #[command(name = CONTEXT_SUBCOMMAND)]
    Context,
}
