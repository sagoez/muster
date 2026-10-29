use std::{
    cell::Cell,
    env,
    io::{self, Write},
    os::unix::process::CommandExt as _,
    panic,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, id as current_pid},
    sync::Arc,
    time::SystemTime,
};

use clap::{CommandFactory, Parser};
use crossbeam_channel::Sender;
use directories::BaseDirs;
use muster::{
    adapter::{
        cli::{
            self, Action, ActionReport, AgentCommand, Announce, Args, AttachEnd, CheckOutcome,
            CliError, CliExit, Command, CoordinationCommand, HooksCommand, HostTerminal,
            InternalHookCommand, KvCommand, NoteCommand, NoticeView, Presenter, ProcessArgs,
            ProjectView, ProjectsCommand, Report, RunArgs, ServiceClient, ServiceCommand,
            ServiceStopped, SettleWatch, TodoCommand, VALID_SUFFIX, agent, output, process,
        },
        config::{
            self, SqliteCoordinationStore, YamlAgentSessionStore, YamlProjectRegistry,
            YamlSettingsStore, starter_workspace,
        },
        holder::{ExecHolderLauncher, HolderDirs, HolderRunner, run_holder},
        hooks::{HookError, ProviderHooks},
        mcp,
        metrics::SysinfoMetrics,
        notifier::DesktopNotifier,
        path::absolutize,
        private_dir::prepare_state_dir,
        process_identity::LocalProcessIdentity,
        pty::{SHELL_COMMAND_FLAG, SHELL_PROGRAM},
        service::{
            NotifyConfigWatcher, ProcessAction, ProjectPorts, ProjectStatus, ServiceDeps,
            ServiceError, ServiceEvent, ServiceRuntime, Singleton, protocol, redirect_stderr,
        },
    },
    constants::{APP_NAME, APP_VERSION, MUSTER_AGENT_LAUNCH_TOKEN_ENV, WORKSPACE_FILE_NAME},
    domain::{
        agent_session::{AgentProcessId, AgentSessionId, LaunchToken},
        coordination::Author,
        port::{
            AgentSessionStore, ConfigWatcher, CoordinationStore, Notifier, ProcessMetrics,
            ProjectRegistry, SettingsStore,
        },
        value::{DetachKey, ProcessName, ProcessRef},
    },
    error::{MusterError, Result},
};
use serde::Serialize;

/// What a service started beside a running one records before it exits, so
/// each start that lost the race is visible in the service log.
const SERVICE_ALREADY_RUNNING: &str =
    "another muster service already runs for this state directory; this one exits";
/// Environment variable controlling the XDG configuration root.
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// XDG configuration root under home when the environment does not override.
const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// The flag that asks a command for JSON output.
const JSON_FLAG: &str = "--json";
/// Report titles for the styled CLI output boxes.
const INIT_TITLE: &str = "muster init";
const CHECK_TITLE: &str = "muster check";
const PROJECTS_TITLE: &str = "muster projects";
const NOTE_TITLE: &str = "muster note";
const TODO_TITLE: &str = "muster todo";
const KV_TITLE: &str = "muster kv";
const COORDINATION_TITLE: &str = "muster coordination";
const COMPLETIONS_TITLE: &str = "muster completions";
const HOOKS_TITLE: &str = "muster hooks";

/// Codex requires user approval before executing a new hook command.
const CODEX_HOOK_APPROVAL_NOTICE: &str =
    "Codex: approve the Muster hook with /hooks before sessions can be resumed.";
/// What `muster down --agents` asks before it stops the agents.
const STOP_AGENTS_QUESTION: &str = "Stop this project's agents too?";
/// What the plain layout says it could not ask instead.
const STOP_AGENTS_ASKED: &str = "whether to stop the agents";
/// The flag that answers it up front.
const YES_FLAG: &str = "--yes";
/// How `muster down --agents` ends when the person declined to stop them.
const NOTHING_STOPPED: &str = "nothing stopped";
/// Entry point: dispatches the subcommand, or prints `ps` for the current
/// project when there is none.
fn main() {
    restore_terminals_on_panic();
    // When invoked by a shell's completion hook this generates candidates and
    // exits; otherwise it returns and normal parsing proceeds.
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) if error.use_stderr() && json_requested() => {
            emit_line(&output::failure_json(&CliError::Usage(
                error.render().to_string(),
            )));
            std::process::exit(CliExit::Usage.code());
        },
        Err(error) => error.exit(),
    };
    if let Err(error) = dispatch(args) {
        // Every escaping error speaks in the CLI's styled voice; the default
        // Result-from-main path would print the Debug form instead.
        fail_with(&error);
    }
}

/// Installs the top-level panic hook: a terminal `muster attach` holds in
/// raw mode is given back before the panic is printed, so the message is
/// readable and the shell usable. A panic on one of the session's helper
/// threads is held instead, and the session prints it once it has given the
/// terminal back itself.
fn restore_terminals_on_panic() {
    let print = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if cli::hold_helper_panic(info.to_string()) {
            return;
        }
        cli::restore_terminals();
        print(info);
    }));
}

/// Whether the command line asks for JSON output, read before it is parsed
/// so a usage error can be answered in JSON too.
fn json_requested() -> bool {
    std::env::args_os().any(|arg| arg == JSON_FLAG)
}

/// Routes the parsed arguments to their command.
///
/// # Errors
/// Returns an error when a command fails; `main` renders it via [`fail_with`].
fn dispatch(args: Args) -> Result<()> {
    match args.command {
        Some(Command::Ps { all, json }) => run_ps(args.config, all, json),
        Some(Command::Up { json }) => act(
            Presenter::detect(json),
            args.config,
            json,
            Action::Up,
            |_, client, project, watch| process::up(client, project, watch).map(Some),
        ),
        Some(Command::Down { agents, yes, json }) => act(
            asking_first(Presenter::detect(json), json, agents && !yes, |presenter| {
                presenter.can_confirm(STOP_AGENTS_ASKED, YES_FLAG)
            }),
            args.config,
            json,
            Action::Down,
            |presenter, client, project, watch| {
                if agents
                    && !yes
                    && process::agents_may_run(client, project)?
                    && !presenter.confirm(STOP_AGENTS_QUESTION, STOP_AGENTS_ASKED, YES_FLAG)?
                {
                    return Ok(None);
                }
                process::down(client, project, agents, watch).map(Some)
            },
        ),
        Some(Command::Start(target)) => control(args.config, target, ProcessAction::Start),
        Some(Command::Stop(target)) => control(args.config, target, ProcessAction::Stop),
        Some(Command::Restart(target)) => control(args.config, target, ProcessAction::Restart),
        Some(Command::Pause(target)) => control(args.config, target, ProcessAction::Pause),
        Some(Command::Resume(target)) => control(args.config, target, ProcessAction::Resume),
        Some(Command::Kill(target)) => control(args.config, target, ProcessAction::Kill),
        Some(Command::Logs {
            name,
            follow,
            screen,
            json,
        }) => run_logs(args.config, name, follow, screen, json),
        Some(Command::Attach {
            name,
            read_only,
            json,
        }) => run_attach(args.config, name.as_ref(), read_only, json),
        Some(Command::Agent { command }) => run_agent(args.config, command),
        Some(Command::Notices { json }) => run_notices(args.config, json),
        Some(Command::Service { command }) => run_service_command(command),
        Some(Command::Init) => run_init(),
        Some(Command::Check) => run_check(
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Run(run_args)) => run_capture(run_args, args.config),
        Some(Command::Projects { command }) => run_projects(command),
        Some(Command::Note { command }) => run_note(
            command,
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Todo { command }) => run_todo(
            command,
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Kv { command }) => run_kv(
            command,
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Coordination { command }) => run_coordination(
            command,
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Mcp {
            print_config,
            author,
        }) => run_mcp(
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
            print_config,
            author,
        ),
        Some(Command::Hooks { command }) => run_hooks(command),
        Some(Command::Doctor) => run_doctor(
            args.config
                .unwrap_or_else(|| PathBuf::from(WORKSPACE_FILE_NAME)),
        ),
        Some(Command::Completions { shell }) => run_completions(shell),
        Some(Command::Hook { command }) => run_internal_hook(command),
        Some(Command::Holder { spec }) => Ok(run_holder(&spec)?),
        Some(Command::RunService) => run_service(),
        None => run_ps(args.config, false, false),
    }
}

/// The absolute config path of the project a process command acts on: the
/// explicit `--config`, the current project exported into processes,
/// `./muster.yml`, then the first registered project. Checked to exist, so
/// a command run outside any project fails before it starts the service.
///
/// # Errors
/// Returns a [`CliError`] when the registry cannot be read or no config
/// exists at the resolved path.
fn project_config(explicit: Option<&Path>) -> std::result::Result<PathBuf, CliError> {
    let config = cli::resolve_project_config(
        explicit,
        cli::current_project_from_env().as_deref(),
        Path::new(WORKSPACE_FILE_NAME),
        &YamlProjectRegistry,
    )?;
    if config.exists() {
        Ok(config)
    } else {
        Err(CliError::NoProjectConfig(config))
    }
}

/// Writes `text` to stdout. A reader that went away, as `| head` does, ends
/// the command quietly; any other failure to write is reported.
fn emit(text: &str) {
    let mut stdout = anstream::stdout();
    if let Err(error) = stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        end_on_write_error(error);
    }
}

/// Writes `text` and a line break to stdout, as [`emit`] does.
fn emit_line(text: &str) {
    emit(&format!("{text}\n"));
}

/// Ends the command after its output could not be written: quietly with
/// success when the reader closed the pipe, with a failure otherwise.
fn end_on_write_error(error: io::Error) -> ! {
    if error.kind() == io::ErrorKind::BrokenPipe {
        std::process::exit(CliExit::Success.code());
    }
    cli::print_error(APP_NAME, &CliError::Output(error));
    std::process::exit(CliExit::ActionFailed.code());
}

/// Ends the command when a write to the person failed, as [`emit`] does.
fn written(result: io::Result<()>) {
    if let Err(error) = result {
        end_on_write_error(error);
    }
}

/// Prints `value` as JSON with `json`, through `human` otherwise.
fn print_result<T: Serialize>(json: bool, value: &T, human: impl FnOnce(&T)) {
    if !json {
        human(value);
        return;
    }
    match output::json(value) {
        Ok(text) => emit_line(&text),
        Err(error) => fail_command(json, &error),
    }
}

/// Reports `error` as JSON on stdout with `json`, in the CLI's voice on
/// stderr otherwise, and exits with the code its kind documents.
fn fail_command(json: bool, error: &CliError) -> ! {
    fail_framed(Presenter::Plain, json, error)
}

/// Reports `error` as JSON on stdout with `json`, otherwise through
/// `presenter`, which closes a frame it opened, and exits with the code its
/// kind documents.
fn fail_framed(presenter: Presenter, json: bool, error: &CliError) -> ! {
    if let CliError::Output(source) = error
        && source.kind() == io::ErrorKind::BrokenPipe
    {
        std::process::exit(CliExit::Success.code());
    }
    let code = CliExit::from(error.kind()).code();
    if json {
        emit_line(&output::failure_json(error));
    } else {
        written(presenter.fail(error));
    }
    std::process::exit(code);
}

/// Prints `result` of an action into the frame `presenter` opened and exits
/// with a failure when a process did not end up as asked. No report means
/// the person declined to go on, so nothing was done.
fn finish_action(
    presenter: Presenter,
    json: bool,
    result: std::result::Result<Option<ActionReport>, CliError>,
) -> Result<()> {
    match result {
        Ok(None) => {
            written(presenter.conclude(NOTHING_STOPPED));
            Ok(())
        },
        Ok(Some(report)) => {
            print_result(json, &report, |report| {
                written(presenter.finish(&output::action_rows(report)));
            });
            if report.failed() {
                std::process::exit(CliExit::ActionFailed.code());
            }
            Ok(())
        },
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// Runs `run` for `action` on the current project through the service and
/// prints its report through `presenter`: in a frame titled for the action
/// and the project, with a spinner while its processes settle, at a
/// terminal. `run` answers no report when the person declined to go on.
fn act(
    presenter: Presenter,
    config: Option<PathBuf>,
    json: bool,
    action: Action,
    run: impl FnOnce(
        Presenter,
        &ServiceClient,
        &Path,
        &mut dyn SettleWatch,
    ) -> std::result::Result<Option<ActionReport>, CliError>,
) -> Result<()> {
    let project = match project_config(config.as_deref()) {
        Ok(project) => project,
        Err(error) => fail_command(json, &error),
    };
    written(presenter.open(&output::action_title(action, &project)));
    let mut spinners = presenter.spinners();
    let result = ServiceClient::connect()
        .and_then(|client| run(presenter, &client, &project, &mut spinners));
    drop(spinners);
    finish_action(presenter, json, result)
}

/// `presenter`, once `check` says it can ask the question a command needs
/// when `needed`; otherwise the command fails at once, before it reaches
/// the service.
fn asking_first(
    presenter: Presenter,
    json: bool,
    needed: bool,
    check: impl FnOnce(Presenter) -> std::result::Result<(), CliError>,
) -> Presenter {
    if needed && let Err(error) = check(presenter) {
        fail_command(json, &error);
    }
    presenter
}

/// Applies a lifecycle `action` to the process `target` names, or to the one
/// chosen at a terminal when it names none.
fn control(config: Option<PathBuf>, target: ProcessArgs, action: ProcessAction) -> Result<()> {
    let ProcessArgs { name, json } = target;
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        name.is_none(),
        Presenter::can_choose_process,
    );
    act(
        presenter,
        config,
        json,
        action.into(),
        |presenter, client, project, watch| {
            let pick = process::named_or_chosen(project, name.as_ref(), presenter);
            process::control(client, project, pick, action, watch).map(Some)
        },
    )
}

/// Prints the current project's processes, or every loaded project's with
/// `all`.
fn run_ps(config: Option<PathBuf>, all: bool, json: bool) -> Result<()> {
    let result = (|| {
        let project = if all {
            None
        } else {
            Some(project_config(config.as_deref())?)
        };
        let client = ServiceClient::connect()?;
        process::ps(&client, project.as_deref())
    })();
    match result {
        Ok(statuses) => {
            let views: Vec<ProjectView> = statuses.iter().map(ProjectView::from).collect();
            print_result(json, &views, |_| {
                emit_line(&output::render_status(&statuses, all));
            });
            Ok(())
        },
        Err(error) => fail_command(json, &error),
    }
}

/// Prints a process's log or screen, or follows its log until interrupted;
/// the process is chosen at a terminal when `name` names none.
fn run_logs(
    config: Option<PathBuf>,
    name: Option<ProcessRef>,
    follow: bool,
    screen: bool,
    json: bool,
) -> Result<()> {
    let asking = name.is_none();
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        asking,
        Presenter::can_choose_process,
    );
    let project = match project_config(config.as_deref()) {
        Ok(project) => project,
        Err(error) => fail_command(json, &error),
    };
    if asking {
        written(presenter.open(&output::logs_title(&project)));
    }
    let frame_open = Cell::new(asking);
    let pick = |status: &ProjectStatus| {
        let chosen = process::named_or_chosen(&project, name.as_ref(), presenter)(status)?;
        if frame_open.replace(false) {
            written(presenter.conclude(chosen.as_ref()));
        }
        Ok(chosen)
    };
    let result = ServiceClient::connect().and_then(|client| {
        if follow {
            match process::follow_logs(&client, &project, pick, json, &mut io::stdout().lock())? {}
        }
        process::logs(&client, &project, pick, screen)
    });
    let framing = if frame_open.get() {
        presenter
    } else {
        Presenter::Plain
    };
    match result {
        Ok(text) => {
            print_result(json, &text, |text| {
                if text.text().is_empty() || text.text().ends_with('\n') {
                    emit(text.text());
                } else {
                    emit_line(text.text());
                }
            });
            Ok(())
        },
        Err(error) => fail_framed(framing, json, &error),
    }
}

/// Takes over this terminal to show a running process until the detach key,
/// the process's exit or a signal ends the session; the process is chosen
/// at a terminal, before the session takes it over, when `name` names none.
/// The terminal is checked first, so a command without one starts no
/// service.
fn run_attach(
    config: Option<PathBuf>,
    name: Option<&ProcessRef>,
    read_only: bool,
    json: bool,
) -> Result<()> {
    let presenter = Presenter::detect(json);
    let prepared = HostTerminal::stdio().and_then(|terminal| {
        if name.is_none() {
            presenter.can_choose_process()?;
        }
        let project = project_config(config.as_deref())?;
        Ok((terminal, project, detach_key()?))
    });
    let (terminal, project, detach) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    written(presenter.open(&output::attach_title(&project)));
    let result = ServiceClient::connect().and_then(|client| {
        let name = process::running_named_or_chosen(&client, &project, name, presenter)?;
        cli::attach_named(
            client,
            &project,
            name.clone(),
            terminal,
            read_only,
            detach,
            announcer(presenter),
        )
        .map(|end| (name, end))
    });
    match result {
        Ok((name, end)) => finish_attach(presenter, &name, &end),
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// Says an attach's opening line through `presenter`.
fn announcer(presenter: Presenter) -> Announce {
    Box::new(move |line: &str| presenter.step(line))
}

/// The key that detaches `muster attach`, from the user's settings.
///
/// # Errors
/// Returns a [`CliError`] when the settings do not load.
fn detach_key() -> std::result::Result<DetachKey, CliError> {
    Ok(YamlSettingsStore.load()?.detach_key().clone())
}

/// Closes an attach's frame with what became of it once the terminal is
/// back, then exits as it ended: a failed exit fails the command, and a
/// signal that ended the session ends this process by the same signal.
fn finish_attach(presenter: Presenter, name: &ProcessName, end: &AttachEnd) -> Result<()> {
    if let Some(line) = output::attach_end_line(name, end) {
        written(presenter.conclude(&line));
    }
    match end {
        AttachEnd::Signalled(signal) => {
            let _ = signal.resend();
            std::process::exit(CliExit::ActionFailed.code());
        },
        AttachEnd::Exited(outcome) if outcome.is_failure() => {
            std::process::exit(CliExit::ActionFailed.code());
        },
        AttachEnd::Detached | AttachEnd::Exited(_) => Ok(()),
    }
}

/// Opens, closes or reopens an agent session in the current project.
fn run_agent(config: Option<PathBuf>, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::New {
            tool,
            name,
            command,
            resume,
            json,
        } => {
            let name = match name
                .map(|name| {
                    ProcessName::try_new(name.clone()).map_err(|_| CliError::InvalidName(name))
                })
                .transpose()
            {
                Ok(name) => name,
                Err(error) => fail_command(json, &error),
            };
            act(
                Presenter::detect(json),
                config,
                json,
                Action::AgentNew,
                |_, client, project, watch| {
                    agent::new(client, project, tool, name, command, resume, watch).map(Some)
                },
            )
        },
        AgentCommand::Close { name, json } => act(
            Presenter::detect(json),
            config,
            json,
            Action::AgentClose,
            |_, client, project, watch| agent::close(client, project, &name, watch).map(Some),
        ),
        AgentCommand::Reopen { json } => act(
            Presenter::detect(json),
            config,
            json,
            Action::AgentReopen,
            |_, client, project, watch| agent::reopen(client, project, watch).map(Some),
        ),
    }
}

/// Prints the current project's recent notices and the service's.
fn run_notices(config: Option<PathBuf>, json: bool) -> Result<()> {
    let result = project_config(config.as_deref()).and_then(|project| {
        let client = ServiceClient::connect()?;
        Ok((process::notices(&client, &project)?, project))
    });
    match result {
        Ok((notices, project)) => {
            let views: Vec<NoticeView> = notices.iter().map(NoticeView::from).collect();
            print_result(json, &views, |_| {
                show(&output::notices_rows(&project, &notices, SystemTime::now()));
            });
            Ok(())
        },
        Err(error) => fail_command(json, &error),
    }
}

/// Prints `report`, data a command answers with, as plain lines on stdout
/// even at a terminal, so it reads and pastes as it is.
fn show(report: &Report) {
    written(Presenter::Plain.body(report));
}

/// Shows or stops the service, never starting it.
fn run_service_command(command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Status { json } => match ServiceClient::status() {
            Ok(status) => {
                print_result(json, &status, |status| show(&output::service_rows(status)));
                Ok(())
            },
            Err(error) => fail_command(json, &error),
        },
        ServiceCommand::Stop { json } => {
            let presenter = Presenter::detect(json);
            written(presenter.open(&output::service_title()));
            match ServiceClient::stop() {
                Ok(was_running) => {
                    let stopped = ServiceStopped::builder().stopped(was_running).build();
                    print_result(json, &stopped, |_| {
                        written(presenter.finish(&output::service_stop_rows(was_running)));
                    });
                    Ok(())
                },
                Err(error) => fail_framed(presenter, json, &error),
            }
        },
    }
}

/// Prints the shell-specific registration hook lines for completions.
fn run_completions(shell: cli::CompletionShell) -> Result<()> {
    let [hint, line] = cli::registration(shell);
    show(&Report::new(COMPLETIONS_TITLE, vec![
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
            show(&Report::new(HOOKS_TITLE, rows));
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
        // `process_id` is the reporting provider process, used only to keep a
        // nested same-provider launch from overwriting the managed conversation;
        // `parent_process_id` is still accepted for compatibility with previously
        // installed hooks but ignored. See `ProviderHooks::capture`.
        InternalHookCommand::Capture {
            provider,
            process_id,
            ..
        } => {
            ProviderHooks::capture(
                &YamlAgentSessionStore,
                provider,
                process_id,
                std::io::stdin(),
            )?;
        },
        InternalHookCommand::Launch { session, command } => {
            let session =
                AgentSessionId::try_new(session).map_err(|_| HookError::MissingSessionId)?;
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
    // The claim carries this launch's token so a restart resets the reporter
    // instead of being ignored as a different PID.
    let launch_token = env::var_os(MUSTER_AGENT_LAUNCH_TOKEN_ENV).and_then(|value| {
        value
            .to_str()
            .and_then(|value| LaunchToken::try_new(value).ok())
    });
    let process_id =
        AgentProcessId::try_new(current_pid()).map_err(|_| HookError::MissingProviderProcessId)?;
    YamlAgentSessionStore.set_owner_process_id(
        &session,
        process_id,
        LocalProcessIdentity::start_token(process_id),
        launch_token,
    )?;
    let command = format!("{command}\nexit $?");
    let error = ProcessCommand::new(SHELL_PROGRAM)
        .arg(SHELL_COMMAND_FLAG)
        .arg(command)
        .exec();
    Err(error.into())
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
        Ok(rows) => show(&Report::new(PROJECTS_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Reads or writes shared scratchpad notes for the project at `config`. The
/// config path is absolutized so a note keeps the same project identity however
/// the command is invoked. Prints each report line and exits non-zero on failure.
///
/// # Errors
/// This function always exits on error rather than propagating; it returns
/// `Result<()>` to match the dispatch signature.
fn run_note(command: Option<NoteCommand>, config: PathBuf) -> Result<()> {
    let project = absolutize(&config);
    let store = SqliteCoordinationStore::open_default()?;
    match cli::note(command, &store, &project, &Author::human()) {
        Ok(rows) => show(&Report::new(NOTE_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Lists or manages the shared todos for the project at `config`, absolutizing
/// the path for a stable project identity. Prints each report line and exits
/// non-zero on failure.
///
/// # Errors
/// This function always exits on error rather than propagating; it returns
/// `Result<()>` to match the dispatch signature.
fn run_todo(command: Option<TodoCommand>, config: PathBuf) -> Result<()> {
    let project = absolutize(&config);
    let store = SqliteCoordinationStore::open_default()?;
    match cli::todo(command, &store, &project, &Author::human()) {
        Ok(rows) => show(&Report::new(TODO_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Reads or writes the shared key-value state for the project at `config`,
/// absolutizing the path for a stable project identity. Prints each report line
/// and exits non-zero on failure.
///
/// # Errors
/// This function always exits on error rather than propagating; it returns
/// `Result<()>` to match the dispatch signature.
fn run_kv(command: Option<KvCommand>, config: PathBuf) -> Result<()> {
    let project = absolutize(&config);
    let store = SqliteCoordinationStore::open_default()?;
    match cli::kv(command, &store, &project, &Author::human()) {
        Ok(rows) => show(&Report::new(KV_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Exports or restores this project's coordination state as portable YAML.
/// Prints the summary line and exits non-zero on failure.
///
/// # Errors
/// Returns an error if the coordination store cannot be opened.
fn run_coordination(command: CoordinationCommand, config: PathBuf) -> Result<()> {
    let project = absolutize(&config);
    let store = SqliteCoordinationStore::open_default()?;
    match cli::coordination(command, &store, &project) {
        Ok(rows) => show(&Report::new(COORDINATION_TITLE, rows)),
        Err(error) => fail_with(&error),
    }
    Ok(())
}

/// Serves this project's coordination state to agents over stdio as an MCP
/// server, blocking until the connecting agent disconnects. With `print_config`,
/// prints the paste-in config snippet and exits instead of serving. Builds a
/// single-threaded tokio runtime scoped to this subprocess; the live process
/// tools reach the service through [`ServiceClient`], which starts it on demand.
///
/// # Errors
/// Returns an error if the async runtime cannot be built; a server failure exits
/// via `fail_with` rather than propagating.
fn run_mcp(config: PathBuf, print_config: bool, author: Option<String>) -> Result<()> {
    let project = absolutize(&config);
    if print_config {
        let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_NAME));
        println!("{}", mcp::connection_config(&executable, &project));
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let author = Author::agent(author.as_deref().unwrap_or_default());
    let store: Box<dyn CoordinationStore + Send + Sync> =
        Box::new(SqliteCoordinationStore::open_default()?);
    if let Err(error) = runtime.block_on(mcp::serve_stdio(store, project, author)) {
        fail_with(&error);
    }
    Ok(())
}

/// Writes a workspace in the current directory and registers it: the
/// processes asked for at a terminal, the starter workspace otherwise. Prints
/// each report line and exits non-zero on failure, writing nothing when the
/// questions are cancelled.
///
/// # Errors
/// Returns an error if the current directory cannot be determined.
fn run_init() -> Result<()> {
    let presenter = Presenter::detect(false);
    let directory = std::env::current_dir()?;
    let needed = match cli::needs_workspace(&directory) {
        Ok(needed) => needed,
        Err(error) => fail_command(false, &error),
    };
    written(presenter.open(INIT_TITLE));
    let workspace = if needed {
        presenter.new_workspace()
    } else {
        Ok(starter_workspace())
    };
    match workspace.and_then(|workspace| cli::init(&directory, &YamlProjectRegistry, &workspace)) {
        Ok(rows) => written(presenter.finish(&Report::new(INIT_TITLE, rows))),
        Err(error) => fail_framed(presenter, false, &error),
    }
    Ok(())
}

/// Validates the workspace config at `path`, printing a success line or the
/// full error chain, and exiting as invalid input when it does not load.
///
/// # Errors
/// This function always exits rather than propagating; it returns `Result<()>`
/// to match the dispatch signature.
fn run_check(path: PathBuf) -> Result<()> {
    let display = path.display().to_string();
    match cli::check(path) {
        Err(error) => fail_with(&error),
        Ok(CheckOutcome::Valid) => {
            show(&Report::new(CHECK_TITLE, vec![cli::Row::unlabeled(
                cli::RowKind::Ok,
                format!("{display} {VALID_SUFFIX}"),
            )]));
            Ok(())
        },
        Ok(CheckOutcome::Invalid(report)) => {
            show(
                &Report::new(CHECK_TITLE, vec![cli::Row::unlabeled(
                    cli::RowKind::Fail,
                    report,
                )])
                .with_failure(true),
            );
            std::process::exit(CliExit::InvalidInput.code());
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
    written(Presenter::detect(false).show(&cli::doctor_report(&probes)));
    if cli::any_failed(&probes) {
        std::process::exit(CliExit::ActionFailed.code());
    }
    Ok(())
}

/// Adds a command to a project, starts it under the service and attaches
/// this terminal to it. Without a terminal, with `--json`, or when the start
/// failed or the command already ended, it reports the start like `start`
/// instead. The project is `--project`, or found as every process command
/// finds it from the top-level `config`. Its arguments, project and the
/// detach key are checked first and the service reached next, so neither a
/// bad command line nor a service that cannot start leaves anything
/// registered.
fn run_capture(args: RunArgs, config: Option<PathBuf>) -> Result<()> {
    let json = args.json();
    let presenter = Presenter::detect(json);
    let prepared =
        cli::RunPlan::prepare(&args, config.as_deref(), &YamlProjectRegistry).and_then(|plan| {
            let attach = match (json, HostTerminal::stdio()) {
                (false, Ok(terminal)) => Some((terminal, detach_key()?)),
                _ => None,
            };
            Ok((plan, attach))
        });
    let (plan, attach) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    let (project, name) = (plan.project().clone(), plan.spec().name().clone());
    written(presenter.open(&output::action_title(Action::Run, &project)));
    let mut spinners = presenter.spinners();
    let started = ServiceClient::connect().and_then(|client| {
        plan.carry_out(&YamlProjectRegistry, &client, &mut spinners)
            .map(|report| (client, report))
    });
    drop(spinners);
    let (client, report) = match started {
        Ok(started) => started,
        Err(error) => fail_framed(presenter, json, &error),
    };
    let Some((terminal, detach)) = attach.filter(|_| !report.failed()) else {
        return finish_action(presenter, json, Ok(Some(report)));
    };
    let rows = output::action_rows(&report);
    written(presenter.body(&rows));
    match cli::attach_named(
        client,
        &project,
        name.clone(),
        terminal,
        false,
        detach,
        announcer(presenter),
    ) {
        Ok(end) => finish_attach(presenter, &name, &end),
        Err(CliError::NotRunning(_)) => {
            written(presenter.close(&rows));
            Ok(())
        },
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// The driven adapters a project loaded by the service runs on: the user's
/// own config, session history, desktop and process table.
struct LocalProjectPorts;

impl ProjectPorts for LocalProjectPorts {
    fn registry(&self) -> Box<dyn ProjectRegistry> {
        Box::new(YamlProjectRegistry)
    }

    fn agent_session_store(&self) -> Option<Box<dyn AgentSessionStore>> {
        Some(Box::new(YamlAgentSessionStore))
    }

    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        Some(Box::new(DesktopNotifier::new()))
    }

    fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
        Some(Box::new(SysinfoMetrics::default()))
    }

    fn config_watcher(&self, events: Sender<ServiceEvent>) -> Option<Box<dyn ConfigWatcher>> {
        Some(Box::new(NotifyConfigWatcher::new(events)))
    }
}

/// Runs this user's service until a client stops or replaces it, unless one
/// already runs, in which case it returns at once. Its own stderr goes to the
/// service log first, so a failure to start is recorded even when it was
/// spawned with no terminal. Every process runs under a holder launched from
/// this binary, with its logs under the state directory.
fn run_service() -> Result<()> {
    let state_dir = config::state_dir().ok_or(ServiceError::NoStateDir)?;
    prepare_state_dir(&state_dir)?;
    redirect_stderr(&protocol::log_path(&state_dir))?;
    let Some(singleton) = Singleton::acquire(&state_dir)? else {
        eprintln!("{SERVICE_ALREADY_RUNNING} (pid {})", std::process::id());
        return Ok(());
    };
    let settings = YamlSettingsStore.load()?;
    let dirs = HolderDirs::under(&state_dir);
    let binary = std::env::current_exe()?;
    let launcher = ExecHolderLauncher::builder().binary(binary.clone()).build();
    let runner = HolderRunner::new(
        Box::new(launcher),
        dirs,
        *settings.log_max_size(),
        *settings.scrollback(),
        *settings.color_scheme(),
    );
    let deps = ServiceDeps::builder()
        .version(APP_VERSION.to_string())
        .binary(binary)
        .holders(Arc::new(runner))
        .ports(Box::new(LocalProjectPorts))
        .settings(settings)
        .build();
    Ok(ServiceRuntime::run(singleton, deps)?)
}

/// Prints `error` to stderr in the CLI's styled voice and exits with the
/// code its kind documents: a command's own failure by its kind, a config
/// that does not load as invalid input, anything else as a failed action.
fn fail_with(error: &(dyn std::error::Error + 'static)) -> ! {
    cli::print_error(APP_NAME, error);
    let exit = if let Some(error) = error.downcast_ref::<CliError>() {
        CliExit::from(error.kind())
    } else if let Some(MusterError::Config(_)) = error.downcast_ref::<MusterError>() {
        CliExit::InvalidInput
    } else {
        CliExit::ActionFailed
    };
    std::process::exit(exit.code());
}

#[cfg(test)]
mod tests {
    use muster::adapter::{
        holder::{HOLDER_SUBCOMMAND, SPEC_FLAG},
        service::protocol::SERVICE_SUBCOMMAND,
    };

    use super::*;

    /// The hidden holder entry point takes its spec from its own `--spec`,
    /// leaving the global workspace `--config` alone.
    #[test]
    fn holder_command_reads_its_spec_path() {
        let args =
            Args::try_parse_from(["muster", HOLDER_SUBCOMMAND, SPEC_FLAG, "/run/holder.json"])
                .unwrap();

        assert!(matches!(
            args.command,
            Some(Command::Holder { spec }) if spec.as_os_str() == "/run/holder.json"
        ));
        assert_eq!(args.config, None);
    }

    /// The hidden service command takes no arguments of its own.
    #[test]
    fn service_command_parses() {
        let args = Args::try_parse_from(["muster", SERVICE_SUBCOMMAND]).unwrap();

        assert!(matches!(args.command, Some(Command::RunService)));
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
