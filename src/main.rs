#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(not(unix))]
use std::process::Child;
#[cfg(any(not(unix), test))]
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
};
use std::{path::PathBuf, process::Command as ProcessCommand};

use clap::{CommandFactory, Parser};
use directories::BaseDirs;
use muster::{
    adapter::{
        cli::{
            self, Args, CheckOutcome, Command, HooksCommand, InternalHookCommand, ProjectsCommand,
            RunArgs, VALID_SUFFIX,
        },
        config::{YamlAgentSessionStore, YamlConfigSource, YamlProjectRegistry, YamlSettingsStore},
        hooks::ProviderHooks,
        notifier::DesktopNotifier,
        path::FsPathCompleter,
        process_identity::LocalProcessIdentity,
        pty::PortablePtyRunner,
        tui::{self, Adapters, TerminalGuard},
    },
    application::Workspace,
    constants::{APP_NAME, WORKSPACE_FILE_NAME},
    domain::{
        agent_session::{AgentProcessId, AgentSessionId},
        port::{AgentSessionStore, ConfigSource},
    },
    error::Result,
};

/// Environment variable controlling the XDG configuration root.
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// XDG configuration root under home when the environment does not override.
const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// Exit code used when a command finds problems to report but does not itself
/// fail. Shared by `muster check` and `muster doctor`.
const FINDINGS_EXIT_CODE: i32 = 1;
/// Exit code used when a command encounters a fatal error.
const FAILURE_EXIT_CODE: i32 = 1;
/// Report titles for the styled CLI output boxes.
const INIT_TITLE: &str = "muster init";
const CHECK_TITLE: &str = "muster check";
const PROJECTS_TITLE: &str = "muster projects";
const COMPLETIONS_TITLE: &str = "muster completions";
const HOOKS_TITLE: &str = "muster hooks";

/// Codex requires user approval before executing a new hook command.
const CODEX_HOOK_APPROVAL_NOTICE: &str =
    "Codex: approve the Muster hook with /hooks before sessions can be resumed.";
/// Prefix for the exclusive readiness file that holds a Windows command shell
/// until its ownership is durable.
#[cfg(any(not(unix), test))]
const WINDOWS_LAUNCH_GATE_PREFIX: &str = "muster-agent-launch-";
/// Content that releases a Windows command shell after ownership persistence.
#[cfg(any(not(unix), test))]
const WINDOWS_LAUNCH_GATE_READY: &[u8] = b"ready";
/// Content that stops a Windows command shell without starting its provider.
#[cfg(any(not(unix), test))]
const WINDOWS_LAUNCH_GATE_CANCELLED: &[u8] = b"cancelled";

/// Owns a Windows process tree that must die with the PTY-side launcher.
#[cfg(windows)]
struct WindowsProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsProcessTree {
    /// Creates a job whose children are killed when this launcher exits.
    ///
    /// # Errors
    /// Returns the Windows error when the job cannot be created or configured.
    fn create() -> std::io::Result<Self> {
        use std::{mem::size_of, ptr};

        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE},
            System::JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
        };

        // SAFETY: Null security attributes and name request a new unnamed job.
        // Windows returns either a valid owned handle or null, which is checked.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job == HANDLE::default() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is initialized, lives for this synchronous call, and
        // its pointer and byte length exactly match the requested information class.
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            // SAFETY: `job` is the valid owned handle created immediately above.
            let _ = unsafe { CloseHandle(job) };
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { job })
    }

    /// Adds the gated command shell and every descendant provider to this job.
    ///
    /// # Errors
    /// Returns the Windows error when the child cannot join the job.
    fn assign(&self, child: &Child) -> std::io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let child_handle = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        // SAFETY: `self.job` remains owned by this value and `child_handle`
        // names the live child process for this synchronous assignment.
        let assigned = unsafe { AssignProcessToJobObject(self.job, child_handle) };
        (assigned != 0)
            .then_some(())
            .ok_or_else(std::io::Error::last_os_error)
    }
}

#[cfg(windows)]
impl Drop for WindowsProcessTree {
    /// Closes the sole job handle, causing Windows to terminate its process tree.
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: `job` is uniquely owned by this RAII guard and is closed once.
        let _ = unsafe { CloseHandle(self.job) };
    }
}

/// Holds a Windows command shell before it can start the provider.
#[cfg(any(not(unix), test))]
struct WindowsLaunchGate {
    path: PathBuf,
    file: File,
}

#[cfg(any(not(unix), test))]
impl WindowsLaunchGate {
    /// Creates an exclusive empty readiness file for one pending command shell.
    ///
    /// # Errors
    /// Returns an error when the temporary readiness file cannot be created.
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "{WINDOWS_LAUNCH_GATE_PREFIX}{}",
            uuid::Uuid::new_v4()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    /// Returns the PowerShell readiness wait that precedes the Windows command.
    ///
    /// # Errors
    /// Returns an error when the temporary path cannot be represented safely.
    fn powershell_wait_command(&self) -> std::io::Result<String> {
        let path = self.path.to_str().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agent launch gate path is not valid UTF-8",
            )
        })?;
        let path = path.replace('\'', "''");
        Ok(format!(
            "powershell.exe -NoProfile -NonInteractive -Command \"$gate = '{path}'; while ($true) {{ $state = [System.IO.File]::ReadAllText($gate); if ($state -eq 'ready') {{ exit 0 }}; if ($state -eq 'cancelled') {{ exit 1 }}; Start-Sleep -Milliseconds 10 }}\""
        ))
    }

    /// Releases the command shell after its PID is stored in session state.
    ///
    /// # Errors
    /// Returns an error when the readiness file cannot be updated.
    fn release(&mut self) -> std::io::Result<()> {
        self.write_state(WINDOWS_LAUNCH_GATE_READY)
    }

    /// Stops the command shell when its durable ownership could not be saved.
    fn cancel(&mut self) -> std::io::Result<()> {
        self.write_state(WINDOWS_LAUNCH_GATE_CANCELLED)
    }

    /// Replaces the observable gate state instead of appending a second state.
    fn write_state(&mut self, state: &[u8]) -> std::io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(state)?;
        self.file.sync_all()
    }

    /// Cancels a gated launch and kills a shell that may have observed partial readiness.
    #[cfg(not(unix))]
    fn cancel_and_wait(&mut self, child: &mut Child) {
        let _ = self.cancel();
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(any(not(unix), test))]
impl Drop for WindowsLaunchGate {
    /// Removes the temporary readiness file after the command completes.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Entry point: dispatches to the `run` capture command or, by default, the TUI.
///
/// # Errors
/// Returns an error if the config cannot be loaded or the terminal fails.
fn main() {
    // When invoked by a shell's completion hook this generates candidates and
    // exits; otherwise it returns and normal parsing proceeds.
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();
    if let Err(error) = dispatch(args) {
        // Every escaping error speaks in the CLI's styled voice; the default
        // Result-from-main path would print the Debug form instead.
        fail_with(&error);
    }
}

/// Routes the parsed arguments to their command.
///
/// # Errors
/// Returns an error when a command fails; `main` renders it via [`fail_with`].
fn dispatch(args: Args) -> Result<()> {
    match args.command {
        Some(Command::Init) => run_init(),
        Some(Command::Check) => run_check(
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Run(run_args)) => run_capture(
            run_args,
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Projects { command }) => run_projects(command),
        Some(Command::Hooks { command }) => run_hooks(command),
        Some(Command::Doctor) => run_doctor(
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Completions { shell }) => run_completions(shell),
        Some(Command::Hook { command }) => run_internal_hook(command),
        None => run_tui(args.config),
    }
}

/// Prints the shell-specific registration hook lines for completions.
fn run_completions(shell: cli::CompletionShell) -> Result<()> {
    let [hint, line] = cli::registration(shell);
    cli::print(&cli::Report::new(COMPLETIONS_TITLE, vec![
        cli::Row::unlabeled(cli::RowKind::Dim, hint),
        cli::Row::unlabeled(cli::RowKind::Plain, line),
    ]));
    Ok(())
}

/// Runs user-facing hook setup.
///
/// # Errors
/// Returns an error if the current executable or a provider config cannot be
/// resolved, parsed, or written.
fn run_hooks(command: HooksCommand) -> Result<()> {
    match command {
        HooksCommand::Setup => {
            let executable = std::env::current_exe()?;
            let paths = ProviderHooks::setup(&executable)?;
            let mut rows = vec![cli::Row::unlabeled(
                cli::RowKind::Ok,
                "installed agent session integrations:",
            )];
            rows.extend(paths.iter().map(|path| {
                cli::Row::unlabeled(cli::RowKind::Plain, format!("  {}", path.display()))
            }));
            rows.push(cli::Row::unlabeled(
                cli::RowKind::Hint,
                CODEX_HOOK_APPROVAL_NOTICE,
            ));
            cli::print(&cli::Report::new(HOOKS_TITLE, rows));
        },
    }
    Ok(())
}

/// Receives lifecycle events from supported provider hooks.
///
/// # Errors
/// Returns an error if a Muster-owned hook payload is invalid or cannot be
/// persisted. Hooks from agents outside Muster are ignored.
fn run_internal_hook(command: InternalHookCommand) -> Result<()> {
    match command {
        InternalHookCommand::Capture {
            provider,
            process_id,
            parent_process_id,
        } => {
            ProviderHooks::capture(
                &YamlAgentSessionStore,
                provider,
                process_id,
                parent_process_id,
                std::io::stdin(),
            )?;
        },
        InternalHookCommand::Launch { session, command } => {
            let session = AgentSessionId::try_new(session)
                .map_err(|_| muster::adapter::hooks::HookError::MissingSessionId)?;
            run_agent_launch(session, command)?;
        },
    }
    Ok(())
}

/// Binds the current launcher to `session` before replacing it with `command`.
///
/// # Errors
/// Returns an error when durable ownership cannot be written or the provider
/// command cannot be started.
fn run_agent_launch(session: AgentSessionId, command: String) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;

        let process_id = AgentProcessId::try_new(std::process::id())
            .map_err(|_| muster::adapter::hooks::HookError::MissingProviderProcessId)?;
        YamlAgentSessionStore.set_owner_process_id(
            &session,
            process_id,
            LocalProcessIdentity::start_token(process_id),
            Some(process_id),
        )?;
        let command = format!("{command}\nexit $?");
        let error = ProcessCommand::new("/bin/sh").arg("-c").arg(command).exec();
        Err(error.into())
    }
    #[cfg(not(unix))]
    {
        let mut gate = WindowsLaunchGate::create()?;
        #[cfg(windows)]
        let process_tree = WindowsProcessTree::create()?;
        let command = format!("{} && {command}", gate.powershell_wait_command()?);
        let mut child = ProcessCommand::new("cmd.exe")
            .args(["/D", "/S", "/C", &command])
            .spawn()?;
        #[cfg(windows)]
        if let Err(error) = process_tree.assign(&child) {
            gate.cancel_and_wait(&mut child);
            return Err(error.into());
        }
        let process_id = match AgentProcessId::try_new(child.id()) {
            Ok(process_id) => process_id,
            Err(_) => {
                gate.cancel_and_wait(&mut child);
                return Err(muster::adapter::hooks::HookError::MissingProviderProcessId.into());
            },
        };
        if let Err(error) = YamlAgentSessionStore.set_owner_process_id(
            &session,
            process_id,
            LocalProcessIdentity::start_token(process_id),
            Some(process_id),
        ) {
            gate.cancel_and_wait(&mut child);
            return Err(error.into());
        }
        if let Err(error) = gate.release() {
            gate.cancel_and_wait(&mut child);
            return Err(error.into());
        }
        let status = child.wait()?;
        drop(gate);
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Lists, adds, or removes registered projects. Prints each report line and
/// exits non-zero on failure.
///
/// # Errors
/// Returns an error if the current directory cannot be determined.
fn run_projects(command: Option<ProjectsCommand>) -> Result<()> {
    let registry = YamlProjectRegistry;
    let result = match command {
        None => cli::list(&registry, &std::env::current_dir()?),
        Some(ProjectsCommand::Add { path }) => {
            cli::add(&path.unwrap_or(PathBuf::from(".")), &registry)
        },
        Some(ProjectsCommand::Remove { name }) => cli::remove(&name, &registry),
    };
    match result {
        Ok(rows) => cli::print(&cli::Report::new(PROJECTS_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Scaffolds a starter workspace in the current directory and registers it.
/// Prints each report line and exits non-zero on failure.
///
/// # Errors
/// Returns an error if the current directory cannot be determined.
fn run_init() -> Result<()> {
    let directory = std::env::current_dir()?;
    let registry = YamlProjectRegistry;
    match cli::init(&directory, &registry) {
        Ok(rows) => cli::print(&cli::Report::new(INIT_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Validates the workspace config at `path`, printing a success line or the
/// full error chain, and exiting with `FINDINGS_EXIT_CODE` on failure.
///
/// # Errors
/// This function always exits rather than propagating; it returns `Result<()>`
/// to match the dispatch signature.
fn run_check(path: PathBuf) -> Result<()> {
    let display = path.display().to_string();
    match cli::check(path) {
        Err(error) => fail_with(&error),
        Ok(CheckOutcome::Valid) => {
            cli::print(&cli::Report::new(CHECK_TITLE, vec![cli::Row::unlabeled(
                cli::RowKind::Ok,
                format!("{display} {VALID_SUFFIX}"),
            )]));
            Ok(())
        },
        Ok(CheckOutcome::Invalid(report)) => {
            cli::print(&cli::Report::new(CHECK_TITLE, vec![cli::Row::unlabeled(
                cli::RowKind::Fail,
                report,
            )]));
            std::process::exit(FINDINGS_EXIT_CODE);
        },
    }
}

/// Runs all doctor probes, prints each line, and exits non-zero if any probe
/// failed.
///
/// # Errors
/// Returns an error if the running executable path cannot be resolved.
fn run_doctor(config_path: PathBuf) -> Result<()> {
    let mut probes = vec![
        cli::config_probe(config_path),
        cli::registry_probe(&YamlProjectRegistry),
        cli::sessions_probe(&YamlAgentSessionStore),
    ];
    probes.push(match ProviderHooks::status(&std::env::current_exe()?) {
        Ok(statuses) => cli::hooks_probe(&statuses),
        Err(error) => cli::hooks_probe_error(&error),
    });
    probes.push(cli::clipboard_probe());
    let shell = std::env::var("SHELL").ok();
    let home = BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default();
    // The XDG root the shells actually read: the override when absolute,
    // else the conventional ~/.config.
    let xdg_config = std::env::var_os(XDG_CONFIG_HOME_ENV)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(XDG_CONFIG_HOME_DEFAULT));
    probes.push(cli::completions_probe(shell.as_deref(), &home, &xdg_config));
    cli::print(&cli::doctor_report(&probes));
    if cli::any_failed(&probes) {
        std::process::exit(FINDINGS_EXIT_CODE);
    }
    Ok(())
}

/// Adds a command to a project and runs it in place, reporting a friendly error
/// and exiting non-zero on failure. `config` is the top-level `--config`, used
/// as the target when neither `--project` nor `$MUSTER_PROJECT` is given.
fn run_capture(args: RunArgs, config: PathBuf) -> Result<()> {
    let registry = YamlProjectRegistry;
    if let Err(error) = cli::run(args, config, &registry) {
        fail_with(&error);
    }
    Ok(())
}

/// Composition root for the TUI: resolves and loads the workspace config, wires
/// its adapters, and runs it under a restoring terminal guard.
///
/// # Errors
/// Returns an error if the config cannot be loaded or the terminal fails.
fn run_tui(explicit_config: Option<PathBuf>) -> Result<()> {
    install_panic_hook();
    let registry = YamlProjectRegistry;
    let current_project = cli::current_project_from_env();
    let local_config = PathBuf::from(WORKSPACE_FILE_NAME);
    let config_path = cli::resolve_tui_config(
        explicit_config.as_deref(),
        current_project.as_deref(),
        &local_config,
        &registry,
    )?;
    let config = YamlConfigSource::builder()
        .path(config_path.clone())
        .build();
    let workspace = Workspace::builder()
        .processes(config.load()?.to_processes())
        .build();
    let adapters = Adapters::builder()
        .runner(Box::new(PortablePtyRunner))
        .registry(Box::new(registry))
        .completer(Box::new(FsPathCompleter))
        .notifier(Box::new(DesktopNotifier::new()))
        .settings_store(Box::new(YamlSettingsStore))
        .agent_session_store(Box::new(YamlAgentSessionStore))
        .build();
    // Queried before the guard takes the terminal over: the color reply
    // arrives on stdin, which the input thread owns afterwards.
    let selection_style = tui::detect_selection_style();
    let mut guard = TerminalGuard::new()?;
    tui::run(
        &mut guard,
        workspace,
        adapters,
        config_path,
        selection_style,
    )
}

/// Prints `error` to stderr in the CLI's styled voice and exits with
/// [`FAILURE_EXIT_CODE`].
fn fail_with(error: &dyn std::error::Error) -> ! {
    cli::print_error(APP_NAME, error);
    std::process::exit(FAILURE_EXIT_CODE);
}

/// Installs a panic hook that restores the terminal before delegating to the
/// previous hook, so a panic never leaves the user stuck in raw mode.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = TerminalGuard::restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows launch gate remains closed until durable ownership releases it.
    #[test]
    fn windows_launch_gate_releases_one_owned_command_shell() {
        let mut gate = WindowsLaunchGate::create().unwrap();
        let wait = gate.powershell_wait_command().unwrap();

        assert!(wait.contains("powershell.exe"));
        assert!(wait.contains("ReadAllText"));
        assert!(wait.contains("cancelled"));
        assert_eq!(std::fs::read(&gate.path).unwrap(), b"");

        gate.release().unwrap();
        assert_eq!(
            std::fs::read(&gate.path).unwrap(),
            WINDOWS_LAUNCH_GATE_READY
        );

        gate.cancel().unwrap();
        assert_eq!(
            std::fs::read(&gate.path).unwrap(),
            WINDOWS_LAUNCH_GATE_CANCELLED
        );

        let mut cancelled = WindowsLaunchGate::create().unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(
            std::fs::read(&cancelled.path).unwrap(),
            WINDOWS_LAUNCH_GATE_CANCELLED
        );
    }

    /// The internal launch handshake accepts one opaque provider shell command.
    #[test]
    fn agent_launch_command_accepts_a_shell_expression() {
        let args = Args::try_parse_from([
            "muster",
            "hook",
            "launch",
            "--session",
            "session-id",
            "--",
            "FOO=bar codex | tee agent.log",
        ])
        .unwrap();

        assert!(matches!(
            args.command,
            Some(Command::Hook {
                command: InternalHookCommand::Launch { session, command }
            }) if session == "session-id" && command == "FOO=bar codex | tee agent.log"
        ));
    }
}
