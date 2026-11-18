use clap::Subcommand;

use super::registration::mcp_registration::McpRegistration;

/// The subcommand of `muster mcp` that prints a session's context.
pub const CONTEXT_SUBCOMMAND: &str = "context";
/// The hidden subcommand of `muster mcp` the plugin's tool hooks run.
pub const GUARD_SUBCOMMAND: &str = "guard";

/// What `muster mcp` does instead of serving.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Set muster up with an agent, through the agent's own plugins.
    ///
    /// For claude and codex this runs the agent's own plugin commands, their
    /// output shown, to install muster's plugin from the latest release: its
    /// MCP server, its skill, a SessionStart hook running `muster mcp
    /// context`, and a PreToolUse hook running `muster mcp guard`. muster
    /// edits no config file itself, and fails with the agent's exit code.
    /// For zed, which has no plugin with hooks, it prints the server entry
    /// to add to Zed's settings.
    Add(McpRegistration),
    /// Take muster's plugin out of an agent.
    ///
    /// Runs the agent's own plugin uninstall, its output shown, and exits as
    /// it does; the marketplace stays. For zed this says what to delete from
    /// its settings.
    Remove(McpRegistration),
    /// Print this project's notes, todos and locks for an agent's session.
    ///
    /// muster's plugin runs this when a session starts, resumes, clears or
    /// compacts. It prints a fenced, labelled block of at most a few
    /// thousand bytes, and nothing outside a project or on any failure,
    /// always exiting 0.
    #[command(name = CONTEXT_SUBCOMMAND)]
    Context,
    /// Answer an agent's PreToolUse hook, read on stdin.
    ///
    /// muster's plugin runs this before edits and shell commands. It
    /// refuses an edit to a project's file, or a git or gh command changing
    /// or publishing a checkout, until the agent holds an in-progress todo,
    /// and the git one until it holds the checkout's lock. It only reads,
    /// and prints nothing, letting the call run, for a call touching no
    /// project, for an agent it cannot identify and on any failure, always
    /// exiting 0.
    #[command(name = GUARD_SUBCOMMAND, hide = true)]
    Guard,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use crate::cli::command_line::args::Args;

    /// `mcp add`'s help says the plugin comes from the latest release, as
    /// the marketplace tracks the release branch, not this binary.
    #[test]
    fn the_add_help_names_the_latest_release() {
        let command = Args::command();
        let help = command
            .find_subcommand("mcp")
            .and_then(|mcp| mcp.find_subcommand("add"))
            .and_then(|add| add.get_long_about())
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(
            help.contains("latest release") && !help.contains("this release"),
            "{help}"
        );
    }
}
