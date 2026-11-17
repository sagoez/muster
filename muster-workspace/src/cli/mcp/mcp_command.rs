use clap::Subcommand;

use super::mcp_registration::McpRegistration;

/// The subcommand of `muster mcp` that prints a session's context.
pub const CONTEXT_SUBCOMMAND: &str = "context";
/// The hidden flag muster's plugin hook passes `mcp context`, marking the
/// run as the hook's; it changes nothing in what is printed.
pub const HOOK_FLAG: &str = "hook";

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Set muster up with an agent, through the agent's own plugins.
    ///
    /// For claude and codex this runs the agent's own commands, their output
    /// shown: `plugin marketplace add sagoez/muster`, then Claude Code's
    /// `plugin install muster@muster` or Codex's `plugin add
    /// muster@muster`. The plugin brings muster's MCP server and a
    /// SessionStart hook running `muster mcp context`; muster edits no
    /// config file itself, and fails with the agent's exit code. Codex runs
    /// the hook once you trust it in /hooks. Zed has no plugin with hooks,
    /// so for zed this prints the server entry to add to its settings and
    /// writes nothing.
    Add(McpRegistration),
    /// Take muster's plugin out of an agent.
    ///
    /// Runs Claude Code's `plugin uninstall muster@muster` or Codex's
    /// `plugin remove muster@muster`, its output shown, and exits as it
    /// does; the marketplace stays. For zed this says what to delete from
    /// its settings.
    Remove(McpRegistration),
    /// Print this project's notes and open todos for an agent's session.
    ///
    /// muster's plugin runs this from its SessionStart hook, so each
    /// session, and each one resumed, cleared or compacted, starts knowing
    /// what other agents left, fenced and labelled as information, never
    /// instructions. It prints at most a few thousand bytes, the newest
    /// notes first, and nothing outside a project or on any failure, always
    /// exiting 0 so the hook never fails a session.
    #[command(name = CONTEXT_SUBCOMMAND)]
    Context {
        /// Marks the run as muster's plugin hook; it prints the same.
        #[arg(long = HOOK_FLAG, hide = true)]
        hook: bool,
    },
}
