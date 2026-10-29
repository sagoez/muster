use std::path::PathBuf;

use clap::{
    Parser, Subcommand,
    builder::{
        PossibleValuesParser, TypedValueParser,
        styling::{AnsiColor, Styles},
    },
};

use super::{completions::CompletionShell, run::RunArgs};
use crate::{
    adapter::{holder::HOLDER_SUBCOMMAND, service::protocol::SERVICE_SUBCOMMAND},
    constants::APP_NAME,
    domain::{
        coordination::TodoStatus,
        process::AgentTool,
        value::{CommandLine, CommandLineError, ProcessRef},
    },
};

/// Help styling matching the CLI's report palette: cyan structure, green
/// literals, yellow placeholders.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Yellow.on_default());

/// What the main help says after the commands: the exit codes and the JSON
/// conventions every process command keeps. A test pins the codes to
/// [`CliExit`](super::CliExit).
const AFTER_HELP: &str = "\
Exit codes:
    0  success
    1  a process did not end up as asked, or the action failed
    2  the command line could not be parsed, or a question was cancelled
    3  the muster service is unavailable or runs a newer version
    4  an unknown project or process name, or an invalid config
  130  interrupted with Ctrl-C while it waited at a terminal

With --json, results and failures print as JSON on stdout. Object keys are
snake_case, enum values are kebab-case, durations are whole seconds, and times
are RFC 3339 in UTC.";

/// Parses the agent `muster agent new` opens, offering every preset and
/// `custom` in help and naming them all on a typo.
fn agent_tool_parser() -> impl TypedValueParser<Value = AgentTool> {
    PossibleValuesParser::new(AgentTool::options().map(AgentTool::protocol_token))
        .try_map(|token| token.parse::<AgentTool>())
}

/// Parses a shell command, refusing a blank one.
///
/// # Errors
/// Returns the [`CommandLineError`] of a blank command.
fn command_line(raw: &str) -> Result<CommandLine, CommandLineError> {
    CommandLine::try_new(raw)
}

/// Command-line arguments. With no subcommand, muster prints `ps` for the
/// current project.
#[derive(Parser)]
#[command(
    name = APP_NAME,
    bin_name = APP_NAME,
    version,
    about = "Run a project's agents and dev processes in the background, and control them from any shell",
    after_help = AFTER_HELP,
    styles = HELP_STYLES
)]
pub struct Args {
    /// The project's muster.yml. Process commands default to $MUSTER_PROJECT,
    /// then ./muster.yml, then the first registered project.
    // Global, so it is recognized before or after a subcommand rather than
    // being swallowed by `run`'s command args.
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. Absent, muster prints `ps` for the current project.
#[derive(Subcommand)]
pub enum Command {
    /// List this project's processes: state, pid, uptime, restarts and usage.
    Ps {
        /// List every project the muster service has loaded.
        #[arg(long)]
        all: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start this project's processes set to `autostart: true`.
    Up {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stop this project's commands and terminals.
    Down {
        /// Stop the project's agents too, after asking at a terminal.
        #[arg(long)]
        agents: bool,
        /// Stop the agents without asking; required with --agents where
        /// nobody can be asked.
        #[arg(short, long, requires = "agents")]
        yes: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start a process.
    Start(ProcessArgs),
    /// Stop a process through its stop policy.
    Stop(ProcessArgs),
    /// Restart a process.
    Restart(ProcessArgs),
    /// Suspend a process, leaving it alive.
    Pause(ProcessArgs),
    /// Resume a paused process.
    Resume(ProcessArgs),
    /// Kill a process at once, skipping its graceful stop.
    Kill(ProcessArgs),
    /// Print a process's output log, kept across its runs and after it exits.
    Logs {
        /// The process, as `muster ps` lists it; quote a name with spaces.
        /// Chosen from a list at a terminal when absent.
        name: Option<ProcessRef>,
        /// Keep printing what the process writes, across restarts, until
        /// interrupted.
        #[arg(short, long, conflicts_with = "screen")]
        follow: bool,
        /// Print the process's current terminal screen instead of its log.
        #[arg(long)]
        screen: bool,
        /// Print the result as JSON; with --follow, one JSON object per log
        /// line.
        #[arg(long)]
        json: bool,
    },
    /// Show a running process in this terminal and type into it.
    ///
    /// The detach key, set in settings.yml, returns to the shell and leaves
    /// the process running.
    Attach {
        /// The process, as `muster ps` lists it; quote a name with spaces.
        /// Chosen from a list at a terminal when absent.
        name: Option<ProcessRef>,
        /// Watch without typing into the process or resizing it.
        #[arg(long)]
        read_only: bool,
        /// Print a refusal or failure as JSON; an attach prints nothing else.
        #[arg(long)]
        json: bool,
    },
    /// Open, close or reopen agent sessions.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Show what the muster service reported about this project: failed
    /// starts, forced stops, crashes.
    Notices {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show or stop the muster service. Processes keep running either way.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Create a muster.yml here and register this folder as a project.
    ///
    /// At a terminal it asks for each process; elsewhere it writes a starter
    /// with one terminal.
    Init,
    /// Validate the workspace config and exit non-zero on problems.
    Check,
    /// Register a command in a project, start it and attach to it.
    ///
    /// Without a terminal, or with --json, the start is reported instead.
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
    /// Serve this project to agents as an MCP server over stdio: its processes,
    /// agent sessions and shared coordination state (spawned as a child
    /// process by a connecting agent).
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
    /// Diagnose config, projects, sessions, hooks, and completions.
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
    /// Internal: runs one process holder from the spec file it is handed.
    #[command(name = HOLDER_SUBCOMMAND, hide = true)]
    Holder {
        /// The holder's spec, written by the launcher and deleted once read.
        #[arg(long, value_name = "PATH")]
        spec: PathBuf,
    },
    /// Internal: runs this user's service in the current process, unless one
    /// already runs.
    #[command(name = SERVICE_SUBCOMMAND, hide = true)]
    RunService,
}

/// The process a lifecycle command acts on.
#[derive(clap::Args)]
pub struct ProcessArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    pub name: Option<ProcessRef>,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Agent session actions under `muster agent`.
#[derive(Subcommand)]
pub enum AgentCommand {
    /// Open and start a new agent session.
    New {
        /// The agent to run; `custom` runs the agent --command names.
        #[arg(value_parser = agent_tool_parser(), ignore_case = true)]
        tool: AgentTool,
        /// Its name; generated from the agent when absent.
        #[arg(long)]
        name: Option<String>,
        /// The command that starts the agent. Required for `custom`; for a
        /// preset it replaces the default command.
        #[arg(long, value_parser = command_line)]
        command: Option<CommandLine>,
        /// The command that resumes a closed conversation. Put `{session_id}`
        /// where the agent's conversation id belongs, as a whole unquoted
        /// word; without it the id is appended.
        #[arg(long, value_name = "TEMPLATE", value_parser = command_line)]
        resume: Option<CommandLine>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Close an agent session, stopping its process.
    Close {
        /// The session, as `muster ps` lists it; quote a name with spaces.
        name: ProcessRef,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reopen the most recently closed agent session.
    Reopen {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Service actions under `muster service`.
#[derive(Subcommand)]
pub enum ServiceCommand {
    /// Show whether the service runs, its version and what it has loaded,
    /// without starting it.
    Status {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stop the service. Every process keeps running under its holder, and
    /// the next muster command starts the service again.
    Stop {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
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

    /// Appends the help of `command`, invoked as `invocation`, and of each of
    /// its visible subcommands to `helps`.
    fn collect_help(command: &mut clap::Command, invocation: &str, helps: &mut Vec<String>) {
        helps.push(format!("$ {invocation} --help\n{}", command.render_help()));
        for sub in command
            .get_subcommands_mut()
            .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        {
            let invocation = format!("{invocation} {}", sub.get_name());
            collect_help(sub, &invocation, helps);
        }
    }

    /// Every visible command's help, so a changed flag, name or description
    /// shows up in review.
    #[test]
    fn cli_args_snapshot() {
        use clap::CommandFactory;

        let mut command = Args::command();
        command.build();
        let mut helps = Vec::new();
        collect_help(&mut command, APP_NAME, &mut helps);

        insta::assert_snapshot!(helps.join("\n\n"));
    }

    /// A name with spaces or an accent reaches a lifecycle command whole and
    /// in normal form C.
    #[test]
    fn names_with_spaces_and_unicode_parse_as_one_argument() {
        let args = Args::try_parse_from(["muster", "stop", "dev server", "--json"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Stop(ProcessArgs { name: Some(name), json: true })) if name.as_ref() == "dev server"
        ));
        let args = Args::try_parse_from(["muster", "logs", "cafe\u{301}", "-f"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Logs { name: Some(name), follow: true, .. }) if name.as_ref() == "caf\u{e9}"
        ));
        assert!(Args::try_parse_from(["muster", "start", "  "]).is_err());
        assert!(Args::try_parse_from(["muster", "logs", "api", "-f", "--screen"]).is_err());
        let args = Args::try_parse_from(["muster", "attach", "dev server", "--read-only"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Attach { name: Some(name), read_only: true, json: false }) if name.as_ref() == "dev server"
        ));
    }

    /// A lifecycle, log or attach command may leave its name out, to be
    /// chosen from a list; `--yes` only answers `down --agents`.
    #[test]
    fn a_name_may_be_left_to_choose_and_yes_belongs_to_agents() {
        for command in ["start", "stop", "restart", "logs", "attach"] {
            assert!(
                Args::try_parse_from(["muster", command]).is_ok(),
                "{command}"
            );
        }
        let args = Args::try_parse_from(["muster", "down", "--agents", "--yes"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Down {
                agents: true,
                yes: true,
                json: false
            })
        ));
        assert!(Args::try_parse_from(["muster", "down", "--yes"]).is_err());
    }

    /// A typo in the agent's name names every agent `agent new` can open,
    /// and any case of a valid name is accepted.
    #[test]
    fn an_unknown_agent_lists_every_valid_one() {
        let error = match Args::try_parse_from(["muster", "agent", "new", "cluade"]) {
            Err(error) => error.render().to_string(),
            Ok(_) => panic!("a typo parses"),
        };
        for tool in AgentTool::options() {
            assert!(error.contains(tool.protocol_token()), "{error}");
        }
        assert!(matches!(
            Args::try_parse_from(["muster", "agent", "new", "Claude"]).map(|args| args.command),
            Ok(Some(Command::Agent {
                command: AgentCommand::New {
                    tool: AgentTool::Claude,
                    ..
                }
            }))
        ));
    }

    /// A custom agent takes its command and an optional resume template; an
    /// empty command is refused at parse time.
    #[test]
    fn a_custom_agent_takes_a_command_and_a_resume_template() {
        let parsed = Args::try_parse_from([
            "muster",
            "agent",
            "new",
            "custom",
            "--command",
            "mycodex",
            "--resume",
            "mycodex resume {session_id}",
        ])
        .map(|args| args.command);

        let Ok(Some(Command::Agent {
            command:
                AgentCommand::New {
                    tool,
                    command,
                    resume,
                    ..
                },
        })) = parsed
        else {
            panic!("a custom agent parses");
        };
        assert_eq!(tool, AgentTool::Custom);
        assert_eq!(command.unwrap().as_ref(), "mycodex");
        assert_eq!(resume.unwrap().as_ref(), "mycodex resume {session_id}");
        assert!(
            Args::try_parse_from(["muster", "agent", "new", "custom", "--command", " "]).is_err()
        );
    }

    /// The exit codes the help documents are the ones commands exit with.
    #[test]
    fn the_documented_exit_codes_match() {
        use strum::IntoEnumIterator;

        use crate::adapter::cli::CliExit;

        for exit in CliExit::iter() {
            assert!(
                AFTER_HELP.contains(&format!("\n{:>5}  ", exit.code())),
                "{exit:?} is documented, aligned with the others"
            );
        }
        assert_eq!(
            AFTER_HELP.matches("\n  ").count(),
            CliExit::iter().count(),
            "no undocumented code is listed"
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
