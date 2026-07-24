use std::path::PathBuf;

use clap::{Parser, Subcommand};

use super::run::RunArgs;
use crate::domain::process::AgentTool;

/// Command-line arguments. With no subcommand, muster launches its TUI.
#[derive(Parser)]
#[command(about = "A terminal workspace for running CLI agents and dev processes")]
pub struct Args {
    /// Path to the workspace config file. Global, so it is recognized before or
    /// after a subcommand rather than being swallowed by `run`'s command args.
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. Absent, muster runs the TUI.
#[derive(Subcommand)]
pub enum Command {
    /// Register a command in a project, then run it.
    Run(RunArgs),
    /// Install provider integrations used to preserve native agent sessions.
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
    /// Internal provider-hook receiver.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: InternalHookCommand,
    },
}

/// User-facing lifecycle-integration commands.
#[derive(Subcommand)]
pub enum HooksCommand {
    /// Install idempotent session-ID hooks/plugins for supported agents.
    Setup,
}

/// Commands invoked by installed provider integrations.
#[derive(Subcommand)]
pub enum InternalHookCommand {
    /// Capture a provider session ID from JSON on standard input.
    Capture {
        /// Provider integration that emitted this lifecycle event.
        #[arg(long)]
        provider: AgentTool,
        /// Parent provider process that invoked the capture hook.
        #[arg(long)]
        process_id: u32,
        /// Parent of the provider process, when the provider was launched by a shell wrapper.
        #[arg(long)]
        parent_process_id: Option<u32>,
    },
    /// Bind a durable session to this process, then start its provider command.
    Launch {
        /// Stable Muster identity of the session being launched.
        #[arg(long)]
        session: String,
        /// Original provider command, preserved as one shell expression.
        #[arg(last = true, allow_hyphen_values = true)]
        command: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    /// The global config flag parses before and after a subcommand.
    #[test]
    fn global_config_flag_parses_anywhere() {
        let before = Args::try_parse_from(["muster", "-c", "x.yml", "run", "--", "ls"]).unwrap();
        assert_eq!(
            before.config.as_deref(),
            Some(std::path::Path::new("x.yml"))
        );
        let bare = Args::try_parse_from(["muster"]).unwrap();
        assert!(bare.command.is_none());
    }
}
