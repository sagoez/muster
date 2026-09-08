use std::path::PathBuf;

use clap::{
    Parser, Subcommand,
    builder::styling::{AnsiColor, Styles},
};

use super::{completions::CompletionShell, run::RunArgs};
use crate::{
    constants::APP_NAME,
    domain::{coordination::TodoStatus, process::AgentTool},
};

/// Help styling matching the CLI's report palette: cyan structure, green
/// literals, yellow placeholders.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Yellow.on_default());

/// Command-line arguments. With no subcommand, muster launches its TUI.
#[derive(Parser)]
#[command(
    name = APP_NAME,
    bin_name = APP_NAME,
    version,
    about = "A terminal workspace for running CLI agents and dev processes",
    styles = HELP_STYLES
)]
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
    /// Create a starter muster.yml here and register this folder as a project.
    Init,
    /// Validate the workspace config and exit non-zero on problems.
    Check,
    /// Register a command in a project, then run it.
    Run(RunArgs),
    /// List registered projects, or add and remove them.
    Projects {
        #[command(subcommand)]
        command: Option<ProjectsCommand>,
    },
    /// Read or write shared scratchpad notes for the current project.
    Note {
        #[command(subcommand)]
        command: Option<NoteCommand>,
    },
    /// List or manage the current project's shared todos.
    Todo {
        #[command(subcommand)]
        command: Option<TodoCommand>,
    },
    /// Read or write the current project's shared key-value state.
    Kv {
        #[command(subcommand)]
        command: Option<KvCommand>,
    },
    /// Export or restore this project's coordination state as portable YAML.
    Coordination {
        #[command(subcommand)]
        command: CoordinationCommand,
    },
    /// Serve this project's coordination state to agents as an MCP server over
    /// stdio (spawned as a child process by a connecting agent).
    Mcp {
        /// Print the JSON config snippet to add muster to an agent's MCP servers,
        /// then exit instead of serving.
        #[arg(long)]
        print_config: bool,
        /// Identity this server stamps on coordination writes (the connecting
        /// agent's name), so entries are attributable. Defaults to a generic
        /// agent label.
        #[arg(long = "as", value_name = "NAME")]
        author: Option<String>,
    },
    /// Install provider integrations used to preserve native agent sessions.
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
    /// Diagnose config, projects, sessions, hooks, clipboard, and completions.
    Doctor,
    /// Print the shell hook that enables completions.
    Completions {
        /// Shell to print the hook for.
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Internal provider-hook receiver.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: InternalHookCommand,
    },
}

/// Registry management actions under `muster projects`.
#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// Register an existing folder containing a muster.yml.
    Add {
        /// Folder to register (defaults to the current directory).
        path: Option<PathBuf>,
    },
    /// Unregister a project by name; files on disk are untouched.
    Remove {
        /// Registered project name.
        name: String,
    },
}

/// Scratchpad actions under `muster note`. Absent, notes are listed.
#[derive(Subcommand)]
pub enum NoteCommand {
    /// Print a note's body by key.
    Get {
        /// Note key.
        key: String,
    },
    /// Create or replace a note.
    Set {
        /// Note key.
        key: String,
        /// Markdown body.
        body: String,
    },
    /// Delete a note by key.
    Rm {
        /// Note key.
        key: String,
    },
}

/// Todo actions under `muster todo`. Absent, todos are listed.
#[derive(Subcommand)]
pub enum TodoCommand {
    /// Add a new pending todo.
    Add {
        /// What the task is.
        title: String,
        /// Id of a todo this one depends on (repeatable).
        #[arg(long = "dep")]
        deps: Vec<String>,
    },
    /// Set a todo's status: pending, in-progress, or done.
    Status {
        /// Todo id.
        id: String,
        /// New status.
        status: TodoStatus,
    },
    /// Delete a todo by id.
    Rm {
        /// Todo id.
        id: String,
    },
}

/// Key-value actions under `muster kv`. Absent, entries are listed.
#[derive(Subcommand)]
pub enum KvCommand {
    /// Print a value by key.
    Get {
        /// Entry key.
        key: String,
    },
    /// Create or replace a value.
    Set {
        /// Entry key.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete an entry by key.
    Rm {
        /// Entry key.
        key: String,
    },
}

/// Snapshot actions under `muster coordination`.
#[derive(Subcommand)]
pub enum CoordinationCommand {
    /// Write this project's scratchpads, todos, and key-values to a YAML file.
    Export {
        /// File to write.
        file: PathBuf,
    },
    /// Restore entries from a YAML file, preserving ids, authors, and timestamps.
    /// Entries merge; those absent from the file are left alone.
    Import {
        /// File to read.
        file: PathBuf,
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

    /// Completions and usage must register the binary name, not the package
    /// name muster-workspace, or every emitted shell hook is inert.
    #[test]
    fn the_command_is_named_after_the_binary() {
        use clap::CommandFactory;

        assert_eq!(Args::command().get_name(), APP_NAME);
        assert_eq!(Args::command().get_bin_name(), Some(APP_NAME));
        assert_eq!(
            Args::command().get_version(),
            Some(env!("CARGO_PKG_VERSION")),
            "muster --version must work on installed builds"
        );
    }

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
