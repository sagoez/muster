use std::path::PathBuf;

use clap::Subcommand;

use super::{
    add_args::AddArgs,
    args::{HOOK_SHELLS, one_of, pid},
    completions::CompletionShell,
    config_subcommand::ConfigCommand,
    coordination_command::CoordinationCommand,
    kv_command::KvCommand,
    lock_command::LockCommand,
    log_filter_args::LogFilterArgs,
    note_command::NoteCommand,
    open_args::OpenArgs,
    process_args::ProcessArgs,
    projects_command::ProjectsCommand,
    run_args::RunArgs,
    service_command::ServiceCommand,
    set_args::SetArgs,
    shell_init::SHELL_INIT_SUBCOMMAND,
    start_args::StartArgs,
    todo_command::TodoCommand,
    wait_args::WaitArgs,
};
use crate::{
    adapter::{
        client::PageSize,
        holder::HOLDER_SUBCOMMAND,
        service::protocol::SERVICE_SUBCOMMAND,
        shell::{HookShell, SHELL_REPORT_SUBCOMMAND},
    },
    domain::{
        occupancy::{LoadStart, ReportSeq},
        value::{EpochNanos, Pid, ProcessRef},
    },
};

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
    /// Show a running process in a new pane of the terminal workspace this
    /// shell runs in, beside this one; outside one, attach to it here, as
    /// `muster attach` does.
    ///
    /// The new pane takes the focus, and detaching closes it; a failed attach
    /// stays on screen there.
    Open(OpenArgs),
    /// Show this project's notices: the stack edits agents made, and failed
    /// starts, forced stops and crashes when the service runs. Never starts it.
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
        /// Notes one page of the listing shows, by key; `note get` shows a
        /// body.
        #[arg(long, default_value_t)]
        limit: PageSize,
        /// List from after this cursor, the one an earlier page printed.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
    },
    /// List or manage the current project's shared todos.
    Todo {
        #[command(subcommand)]
        command: Option<TodoCommand>,
        /// Todos one page of the listing shows.
        #[arg(long, default_value_t)]
        limit: PageSize,
        /// List from after this cursor, the one an earlier page printed.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
    },
    /// Read or write the current project's shared key-value state.
    Kv {
        #[command(subcommand)]
        command: Option<KvCommand>,
        /// Entries one page of the listing shows, each value cut short;
        /// `kv get` shows a value whole.
        #[arg(long, default_value_t)]
        limit: PageSize,
        /// List from after this cursor, the one an earlier page printed.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
    },
    /// List the locks agents hold on the current project, or release one.
    Lock {
        #[command(subcommand)]
        command: Option<LockCommand>,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
    },
    /// Export or restore this project's coordination state as portable YAML.
    Coordination {
        #[command(subcommand)]
        command: CoordinationCommand,
        /// Print the result as JSON.
        #[arg(long, global = true)]
        json: bool,
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
    /// Diagnose muster's database, projects, completions, shell hook and
    /// terminal workspace.
    Doctor {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the shell hook that enables completions.
    Completions {
        /// Shell to print the hook for.
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Print the shell hook that starts a project's processes when a shell
    /// enters it.
    ///
    /// They stop once no shell has been in the project for the idle-stop
    /// time. Add `eval "$(muster shell-init bash)"` to ~/.bashrc, the same with zsh
    /// to ~/.zshrc, or `muster shell-init fish | source` to fish's
    /// config.fish. The prompt never waits on muster.
    #[command(name = SHELL_INIT_SUBCOMMAND)]
    ShellInit {
        /// Shell to print the hook for.
        #[arg(value_parser = one_of::<HookShell>(&HOOK_SHELLS))]
        shell: HookShell,
    },
    /// Internal: what the shell hook runs at each prompt to report its shell.
    #[command(name = SHELL_REPORT_SUBCOMMAND, hide = true)]
    ShellReport {
        /// The shell's pid.
        #[arg(long, value_parser = pid)]
        pid: Pid,
        /// When the hook loaded in this shell, in nanoseconds since the epoch.
        #[arg(long)]
        load: LoadStart,
        /// The pid of a process that load spawned, breaking a tie of `--load`.
        #[arg(long, value_parser = pid)]
        load_pid: Pid,
        /// The prompt's number, rising within that load.
        #[arg(long)]
        seq: ReportSeq,
        /// When the shell was first seen, at a prompt, in the folder it is
        /// in, in nanoseconds since the epoch; the report's own start stands
        /// in when the shell cannot read its clock.
        #[arg(long)]
        arrived: Option<EpochNanos>,
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
