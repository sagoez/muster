use std::{error::Error, fmt::Display, path::PathBuf, str::FromStr, sync::LazyLock};

use clap::{
    Command as ClapCommand, CommandFactory, Parser, Subcommand,
    builder::{
        PossibleValuesParser, TypedValueParser,
        styling::{AnsiColor, Styles},
    },
};
use clap_complete::{ArgValueCandidates, CompletionCandidate};
use strum::IntoEnumIterator;

use super::{
    cli_ports::CliPorts,
    completions::CompletionShell,
    error::CliError,
    process_flags::{ProcessFlags, READY_GROUP},
};
use crate::{
    adapter::{
        holder::{HOLDER_SUBCOMMAND, LineFilter, LineLimit, LineMatcher, LogQuery, MAX_LOG_LINES},
        service::{StateGoal, protocol::SERVICE_SUBCOMMAND},
    },
    constants::APP_NAME,
    domain::{
        config::SettingKey,
        coordination::TodoStatus,
        process::{PatternKind, RestartPolicy, StopSignal},
        value::{
            CommandLine, CommandLineError, LocalPort, LocalUrl, OutputCursor, PositiveDuration,
            ProcessRef, StopGrace,
        },
    },
};

/// The id of the global `--project` argument.
const PROJECT_ARG: &str = "project";
/// Help styling matching the CLI's report palette: cyan structure, green
/// literals, yellow placeholders.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Yellow.on_default());

/// What the main help says after the commands: the exit codes and the JSON
/// conventions every command keeps. A test pins the codes to
/// [`CliExit`](super::CliExit).
const AFTER_HELP: &str = "\
Exit codes:
    0  success
    1  a process did not end up as asked, or the action failed
    2  the command line could not be parsed, or a question was cancelled
    3  the muster service is unavailable or runs a newer version
    4  not a project, an unknown project or process, or an invalid value or file
  130  interrupted with Ctrl-C while it waited at a terminal

With --json, results and failures print as JSON on stdout. Object keys are
snake_case, enum values are kebab-case, durations are seconds, and times are
RFC 3339 in UTC.";

/// What `muster logs --lines` takes, followed by its maximum.
const LINE_LIMIT_FORM: &str = "a whole number from 1 to ";
/// The `muster logs` flags that print the whole raw log or follow it, which
/// a filtered read conflicts with.
const RAW_LOG_FLAGS: [&str; 2] = ["follow", "screen"];

/// The names `config` accepts for a setting.
static SETTING_KEYS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<SettingKey>);
/// The names `--restart` accepts.
static RESTART_POLICIES: LazyLock<Vec<String>> = LazyLock::new(variant_names::<RestartPolicy>);
/// The names `--stop-signal` accepts.
static STOP_SIGNALS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<StopSignal>);
/// The names `muster wait --state` accepts.
static STATE_GOALS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<StateGoal>);

/// The kebab-case name of each variant of `T`.
fn variant_names<T: IntoEnumIterator + Display>() -> Vec<String> {
    T::iter().map(|variant| variant.to_string()).collect()
}

/// Parses one of `names` into `T`, listing them all in help and on a typo.
pub(super) fn one_of<T>(names: &'static [String]) -> impl TypedValueParser<Value = T>
where
    T: FromStr + Clone + Send + Sync + 'static,
    T::Err: Error + Send + Sync + 'static,
{
    PossibleValuesParser::new(names.iter().map(String::as_str)).try_map(|raw| raw.parse::<T>())
}

/// Parses a `--restart` policy.
pub(super) fn restart_parser() -> impl TypedValueParser<Value = RestartPolicy> {
    one_of(&RESTART_POLICIES)
}

/// Parses a `--stop-signal`.
pub(super) fn stop_signal_parser() -> impl TypedValueParser<Value = StopSignal> {
    one_of(&STOP_SIGNALS)
}

/// Parses a `muster wait --state`.
fn state_goal_parser() -> impl TypedValueParser<Value = StateGoal> {
    one_of(&STATE_GOALS)
}

/// Parses a shell command, refusing a blank one.
///
/// # Errors
/// Returns the [`CommandLineError`] of a blank command.
pub(super) fn command_line(raw: &str) -> Result<CommandLine, CommandLineError> {
    CommandLine::try_new(raw)
}

/// Parses a positive duration such as `5s` or `1m 30s`.
///
/// # Errors
/// Returns the parse failure of anything else.
pub(super) fn positive_duration(raw: &str) -> Result<PositiveDuration, String> {
    raw.parse::<PositiveDuration>()
        .map_err(|error| error.to_string())
}

/// Parses a stop grace such as `5s`, at most an hour.
///
/// # Errors
/// Returns the parse failure of anything else.
pub(super) fn stop_grace(raw: &str) -> Result<StopGrace, String> {
    raw.parse::<StopGrace>().map_err(|error| error.to_string())
}

/// Parses a `muster logs --lines`: a whole number from 1 to
/// [`MAX_LOG_LINES`].
///
/// # Errors
/// Returns the form expected for anything else.
fn line_limit(raw: &str) -> Result<LineLimit, String> {
    raw.trim()
        .parse()
        .ok()
        .and_then(|lines| LineLimit::try_new(lines).ok())
        .ok_or_else(|| format!("{LINE_LIMIT_FORM}{MAX_LOG_LINES}"))
}

/// The command line's definition, with `--project` completing the projects
/// `ports` has registered.
pub(super) fn completing_command(ports: &'static dyn CliPorts) -> ClapCommand {
    Args::command().mut_arg(PROJECT_ARG, |arg| {
        arg.add(ArgValueCandidates::new(move || project_candidates(ports)))
    })
}

/// Shell-completion candidates for `--project`: the registered projects'
/// names, read fresh each time so a new project completes at once. Reads an
/// existing database read-only and never creates one.
fn project_candidates(ports: &dyn CliPorts) -> Vec<CompletionCandidate> {
    ports
        .existing_state()
        .ok()
        .flatten()
        .and_then(|state| state.projects().ok())
        .unwrap_or_default()
        .iter()
        .map(|project| CompletionCandidate::new(project.name().as_ref()))
        .collect()
}

/// Command-line arguments. With no subcommand, muster prints `ps` for the
/// current project, or every loaded project outside one.
#[derive(Parser)]
#[command(
    name = APP_NAME,
    bin_name = APP_NAME,
    version,
    about = "Run a project's dev processes in the background, and control them from any shell",
    after_help = AFTER_HELP,
    styles = HELP_STYLES
)]
pub struct Args {
    /// The project: a registered project's name, or a folder (a value with
    /// `/`, or starting with `.` or `~`). Defaults to $MUSTER_PROJECT, then
    /// the registered project containing the current folder.
    // Global, so it is recognized before or after a subcommand rather than
    // being swallowed by `run`'s command words.
    #[arg(short, long, global = true, value_name = "PROJECT")]
    pub project: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. Absent, muster prints `ps`.
#[derive(Subcommand)]
pub enum Command {
    /// List this project's processes: state, pid, uptime, restarts and usage.
    Ps {
        /// List every project the muster service has loaded.
        #[arg(long)]
        all: bool,
        /// Add a COMMAND column with each process's stored command, so an
        /// edit made by an agent shows.
        #[arg(long)]
        commands: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start this project's processes that autostart.
    Up {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stop every process of this project.
    Down {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start a process.
    Start(StartArgs),
    /// Stop a process through its stop policy.
    Stop(ProcessArgs),
    /// Restart a process.
    Restart(StartArgs),
    /// Wait until a process prints a line, accepts a connection, answers a
    /// URL, reaches a state, or passes its readiness check.
    ///
    /// Without a condition, waits for its readiness check, or for it to run
    /// when it has none.
    Wait(WaitArgs),
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
        #[command(flatten)]
        filter: LogFilterArgs,
        /// Print the result as JSON; with --follow, one JSON object per log
        /// line.
        #[arg(long)]
        json: bool,
    },
    /// Show a running process in this terminal and type into it.
    ///
    /// The detach key (`muster config get detach-key`) returns to the shell
    /// and leaves the process running.
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
    /// Add a process to this project, registering the folder as a project
    /// when it is not one yet. Nothing is started.
    Add(AddArgs),
    /// Change a process's definition; a running process takes it at its next
    /// start.
    Set(SetArgs),
    /// Remove processes from this project, stopping those that run.
    Rm {
        /// The processes, as `muster ps` lists them.
        #[arg(required = true)]
        names: Vec<ProcessRef>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show or change muster's settings.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
    },
    /// Write this project's processes to a stack file.
    Export {
        /// The file to write; `-` prints it. Defaults to muster.yml in the
        /// project's folder.
        file: Option<String>,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
        /// Write environment values too; without it each variable is
        /// written with no value, to be filled in before importing.
        #[arg(long)]
        with_env: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Load processes from a stack file into this project, registering the
    /// folder when it is not a project yet.
    Import {
        /// The file to read; `-` reads stdin. Defaults to muster.yml in the
        /// project's folder.
        file: Option<String>,
        /// Replace same-named processes and keep the others.
        #[arg(long, conflicts_with = "replace")]
        merge: bool,
        /// Make the project's processes exactly the file's, stopping and
        /// removing the others.
        #[arg(long)]
        replace: bool,
        /// Print what would change and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Find processes in this folder's package.json, Procfile, justfile,
    /// Makefile or Compose file and add the ones chosen.
    ///
    /// At a terminal it asks which to add; elsewhere it lists them and adds
    /// the suggested ones with --yes.
    Init {
        /// Add the suggested processes without asking.
        #[arg(short, long)]
        yes: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a command to this project, start it and attach to it.
    ///
    /// Without a terminal, or with --json, the start is reported instead.
    Run(RunArgs),
    /// List registered projects, or remove one.
    Projects {
        #[command(subcommand)]
        command: Option<ProjectsCommand>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
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
    /// List the locks agents hold on the current project, or release one.
    Lock {
        #[command(subcommand)]
        command: Option<LockCommand>,
    },
    /// Export or restore this project's coordination state as portable YAML.
    Coordination {
        #[command(subcommand)]
        command: CoordinationCommand,
    },
    /// Serve muster to agents as an MCP server over stdio (spawned as a
    /// child process by a connecting agent). Each call acts on the project
    /// containing this folder, or the one --project or $MUSTER_PROJECT
    /// names, unless it names another.
    Mcp {
        /// Print the JSON config snippet to add muster to an agent's MCP
        /// servers, then exit instead of serving.
        #[arg(long)]
        print_config: bool,
        /// The agent's id on coordination writes and locks; defaults to
        /// MUSTER_AGENT, then herdr's pane, then a random session id.
        #[arg(long = "as", value_name = "NAME")]
        agent: Option<String>,
    },
    /// Diagnose muster's database, projects and completions.
    Doctor,
    /// Print the shell hook that enables completions.
    Completions {
        /// Shell to print the hook for.
        #[arg(value_enum)]
        shell: CompletionShell,
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

/// The process a start or restart acts on, and whether to wait for it.
#[derive(clap::Args)]
pub struct StartArgs {
    /// The process and output form.
    #[command(flatten)]
    pub target: ProcessArgs,
    /// Wait until its new run passes its readiness check, or runs when it has
    /// none.
    #[arg(long)]
    pub wait: bool,
    /// With --wait, the longest to wait, such as 60s; unbounded when absent.
    #[arg(long, requires = "wait", value_name = "DURATION", value_parser = positive_duration)]
    pub timeout: Option<PositiveDuration>,
}

/// `muster wait`: a process and what to wait for.
#[derive(clap::Args)]
pub struct WaitArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    pub name: Option<ProcessRef>,
    /// Wait for an output line containing this text.
    #[arg(long, group = "until", value_name = "TEXT")]
    pub log: Option<String>,
    /// Read --log as a regular expression.
    #[arg(long, requires = "log")]
    pub regex: bool,
    /// Wait until a TCP connection to this localhost port succeeds.
    #[arg(long, group = "until", value_name = "PORT")]
    pub port: Option<LocalPort>,
    /// Wait until this localhost URL answers below 500.
    #[arg(long, group = "until", value_name = "URL")]
    pub http: Option<LocalUrl>,
    /// With --http, the status to wait for instead.
    #[arg(long, requires = "http", value_name = "CODE")]
    pub status: Option<u16>,
    /// Wait for a lifecycle state; `stopped` means not running at all.
    #[arg(long, group = "until", value_parser = state_goal_parser())]
    pub state: Option<StateGoal>,
    /// The longest to wait, such as 60s; unbounded when absent.
    #[arg(long, value_name = "DURATION", value_parser = positive_duration)]
    pub timeout: Option<PositiveDuration>,
    /// Count output after this cursor, from an earlier wait's result, as
    /// well as output after the command started.
    #[arg(long, value_name = "CURSOR")]
    pub since: Option<OutputCursor>,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

/// `muster logs` flags that read the log as stripped, filtered lines rather
/// than the raw bytes; any one of them switches to that read.
#[derive(clap::Args)]
pub struct LogFilterArgs {
    /// Print only the newest N lines, escapes removed (at most 2000).
    #[arg(short = 'n', long, value_name = "N", value_parser = line_limit, conflicts_with_all = RAW_LOG_FLAGS)]
    pub lines: Option<LineLimit>,
    /// Print only output after this cursor, from an earlier `muster wait` or
    /// `muster logs --json`.
    #[arg(long, value_name = "CURSOR", conflicts_with_all = RAW_LOG_FLAGS)]
    pub since: Option<OutputCursor>,
    /// Print only lines containing this text.
    #[arg(long, value_name = "TEXT", conflicts_with_all = RAW_LOG_FLAGS)]
    pub grep: Option<String>,
    /// Read --grep as a regular expression.
    #[arg(long, requires = "grep")]
    pub regex: bool,
    /// Print only error lines, each with its stack trace.
    #[arg(long, conflicts_with = "grep", conflicts_with_all = RAW_LOG_FLAGS)]
    pub errors: bool,
}

impl LogFilterArgs {
    /// Whether any filter flag was given.
    pub fn is_empty(&self) -> bool {
        self.lines.is_none() && self.since.is_none() && self.grep.is_none() && !self.errors
    }

    /// The log read these flags ask for, its pattern compiled first.
    ///
    /// # Errors
    /// Returns [`CliError::InvalidPattern`] for a blank or overlong `--grep`
    /// or a `--regex` that does not compile.
    pub fn query(&self) -> Result<LogQuery, CliError> {
        let filter = match &self.grep {
            Some(grep) => LineFilter::Grep(LineMatcher::of_text(
                grep,
                PatternKind::of_regex(self.regex),
            )?),
            None if self.errors => LineFilter::ErrorsOnly,
            None => LineFilter::All,
        };
        Ok(LogQuery::builder()
            .since(self.since)
            .lines(self.lines.unwrap_or_default())
            .filter(filter)
            .build())
    }
}

/// `muster add`: a new process's definition.
#[derive(clap::Args)]
pub struct AddArgs {
    /// Its name, unique in the project.
    pub name: String,
    /// Its working directory, read from the current folder; stored relative
    /// to the project when inside it. Defaults to the project's folder.
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// A one-line description.
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    #[command(flatten)]
    pub flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
    /// The command, after `--`: one argument is a shell expression, several
    /// are joined as the shell split them.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<String>,
}

/// `muster set`: the changes to a process's definition.
#[derive(clap::Args)]
pub struct SetArgs {
    /// The process, as `muster ps` lists it.
    pub name: ProcessRef,
    /// A new command, as one shell expression.
    #[arg(long, value_name = "CMD", value_parser = command_line, conflicts_with = "words")]
    pub command: Option<CommandLine>,
    /// A new working directory, read from the current folder.
    #[arg(long, value_name = "DIR", conflicts_with = "no_cwd")]
    pub cwd: Option<PathBuf>,
    /// Run in the project's folder again.
    #[arg(long)]
    pub no_cwd: bool,
    /// A new one-line description.
    #[arg(long, value_name = "TEXT", conflicts_with = "no_description")]
    pub description: Option<String>,
    /// Remove the description.
    #[arg(long)]
    pub no_description: bool,
    /// Remove an environment variable (repeatable); one that is not set is
    /// left alone.
    #[arg(long = "unset-env", value_name = "KEY")]
    pub unset_env: Vec<String>,
    /// Remove the readiness check.
    #[arg(long, conflicts_with = READY_GROUP)]
    pub no_ready: bool,
    #[command(flatten)]
    pub flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
    /// A new command, after `--`, joined as `muster add` joins it.
    #[arg(last = true, value_name = "COMMAND")]
    pub words: Vec<String>,
}

/// `muster run`: a command to add, start and attach to.
#[derive(clap::Args)]
pub struct RunArgs {
    /// Its name. Defaults to the command's first word.
    #[arg(short, long)]
    pub name: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
    /// The command, e.g. `-- npm run dev`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub command: Vec<String>,
}

/// Settings actions under `muster config`. Absent, every setting is listed.
#[derive(Subcommand)]
pub enum ConfigCommand {
    /// List every setting and its value.
    List,
    /// Print one setting's value.
    Get {
        /// The setting.
        #[arg(value_parser = one_of::<SettingKey>(&SETTING_KEYS))]
        key: SettingKey,
    },
    /// Change one setting.
    Set {
        /// The setting.
        #[arg(value_parser = one_of::<SettingKey>(&SETTING_KEYS))]
        key: SettingKey,
        /// Its new value, in the form `muster config get` prints.
        value: String,
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

/// Project actions under `muster projects`. Absent, projects are listed.
#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// Stop a project's processes and forget it with its processes and
    /// coordination state; its folder is untouched.
    Rm {
        /// The project's name, or its folder.
        project: String,
        /// Remove without asking; required where nobody can be asked.
        #[arg(short, long)]
        yes: bool,
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
        /// Change it even though an agent holds it.
        #[arg(long)]
        force: bool,
    },
    /// Delete a todo by id.
    Rm {
        /// Todo id.
        id: String,
    },
}

/// Lock actions under `muster lock`. Absent, held locks are listed.
#[derive(Subcommand)]
pub enum LockCommand {
    /// Release a lock whoever holds it, to clear one an agent left behind.
    Release {
        /// Lock name.
        name: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Completions and usage must register the binary name, not the package
    /// name muster-workspace, or every emitted shell hook is inert.
    #[test]
    fn the_command_is_named_after_the_binary() {
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
    fn collect_help(command: &mut ClapCommand, invocation: &str, helps: &mut Vec<String>) {
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
        let mut command = Args::command();
        command.build();
        let mut helps = Vec::new();
        collect_help(&mut command, APP_NAME, &mut helps);

        insta::assert_snapshot!(helps.join("\n\n"));
    }

    /// clap's own consistency checks pass for every command.
    #[test]
    fn the_command_line_is_consistent() {
        Args::command().debug_assert();
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
    }

    /// A lifecycle, log or attach command may leave its name out, to be
    /// chosen from a list; `down` takes no agent flags any more.
    #[test]
    fn a_name_may_be_left_to_choose() {
        for command in ["start", "stop", "restart", "logs", "attach"] {
            assert!(
                Args::try_parse_from(["muster", command]).is_ok(),
                "{command}"
            );
        }
        assert!(Args::try_parse_from(["muster", "down", "--agents"]).is_err());
        assert!(Args::try_parse_from(["muster", "rm"]).is_err());
    }

    /// `add` takes its command after `--` and the shared process flags;
    /// `set` takes the command as a flag or after `--`, never both.
    #[test]
    fn add_and_set_parse_their_definitions() {
        let args = Args::try_parse_from([
            "muster",
            "add",
            "web",
            "--env",
            "PORT=5173",
            "--restart",
            "on-failure",
            "--stop-grace",
            "10s",
            "--no-autostart",
            "--",
            "npm",
            "run",
            "dev",
        ])
        .unwrap();
        let Some(Command::Add(add)) = args.command else {
            panic!("add parses");
        };
        assert_eq!(add.name, "web");
        assert_eq!(add.command, ["npm", "run", "dev"]);
        assert_eq!(add.flags.restart, Some(RestartPolicy::OnFailure));
        assert_eq!(add.flags.autostart(), Some(false));
        assert!(Args::try_parse_from(["muster", "add", "web"]).is_err());
        assert!(
            Args::try_parse_from(["muster", "add", "web", "--restart", "sometimes", "--", "x"])
                .is_err()
        );
        assert!(
            Args::try_parse_from(["muster", "set", "web", "--command", "a", "--", "b"]).is_err()
        );
        assert!(Args::try_parse_from(["muster", "set", "web", "--cwd", "x", "--no-cwd"]).is_err());
    }

    /// A misspelt setting lists every valid key; `config` alone lists them.
    #[test]
    fn config_keys_are_listed_on_a_typo() {
        let error = match Args::try_parse_from(["muster", "config", "get", "detach_key"]) {
            Err(error) => error.render().to_string(),
            Ok(_) => panic!("a typo parses"),
        };
        for key in SettingKey::iter() {
            assert!(error.contains(&key.to_string()), "{error}");
        }
        let args = Args::try_parse_from(["muster", "config", "--json"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Config {
                command: None,
                json: true
            })
        ));
    }

    /// `import` takes one of `--merge` and `--replace`, not both.
    #[test]
    fn import_modes_exclude_each_other() {
        assert!(Args::try_parse_from(["muster", "import", "--merge", "--replace"]).is_err());
        assert!(Args::try_parse_from(["muster", "import", "-", "--replace", "--dry-run"]).is_ok());
    }

    /// The exit codes the help documents are the ones commands exit with.
    #[test]
    fn the_documented_exit_codes_match() {
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

    /// The global project flag parses before and after a subcommand, and is
    /// not swallowed by `run`'s command words.
    #[test]
    fn the_project_flag_parses_anywhere() {
        let before = Args::try_parse_from(["muster", "-p", "shop", "run", "--", "ls"]).unwrap();
        assert_eq!(before.project.as_deref(), Some("shop"));
        let after = Args::try_parse_from(["muster", "ps", "--project", "./web"]).unwrap();
        assert_eq!(after.project.as_deref(), Some("./web"));
        let bare = Args::try_parse_from(["muster"]).unwrap();
        assert!(bare.command.is_none());
    }

    /// The hidden holder entry point takes its spec from its own `--spec`.
    #[test]
    fn the_holder_command_reads_its_spec_path() {
        let args =
            Args::try_parse_from(["muster", HOLDER_SUBCOMMAND, "--spec", "/run/holder.json"])
                .unwrap();

        assert!(matches!(
            args.command,
            Some(Command::Holder { spec }) if spec.as_os_str() == "/run/holder.json"
        ));
        assert_eq!(args.project, None);
        assert!(matches!(
            Args::try_parse_from(["muster", SERVICE_SUBCOMMAND]).map(|args| args.command),
            Ok(Some(Command::RunService))
        ));
    }
}
