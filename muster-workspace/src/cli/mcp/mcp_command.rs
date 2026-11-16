use clap::Subcommand;

use super::mcp_registration::McpRegistration;

/// The subcommand of `muster mcp` that prints a session's context.
pub const CONTEXT_SUBCOMMAND: &str = "context";
/// The hidden flag the hook passes `mcp context`: the hook input is read
/// from stdin, and it marks the hook as muster's own.
pub const HOOK_FLAG: &str = "hook";

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Register muster's MCP server with an agent.
    ///
    /// Runs the agent's own `mcp add` in this folder, launching this muster
    /// binary, unless muster is already registered there: a registration
    /// is never replaced. Either way a SessionStart hook running `muster
    /// mcp context` goes in that scope's hooks file, edited in place:
    /// ~/.claude/settings.json for user, .claude/settings.json for project,
    /// .claude/settings.local.json for local, ~/.codex/hooks.json for Codex,
    /// which runs it once you trust it in /hooks. Zed has no such command or
    /// hook, so for zed this prints the entry to add to its settings and
    /// writes nothing.
    Add(McpRegistration),
    /// Unregister muster's MCP server from an agent.
    ///
    /// Takes muster's SessionStart hook out of the hooks file of the scope
    /// it removes muster from, leaving every other hook, then runs the
    /// agent's own `mcp remove` in this folder and exits as it does, so the
    /// hook never outlives the server. For zed this says what to delete from
    /// its settings.
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
    Context {
        /// Run as muster's hook: read the agent's hook input from stdin.
        #[arg(long = HOOK_FLAG, hide = true)]
        hook: bool,
    },
}
