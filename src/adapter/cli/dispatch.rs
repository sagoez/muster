//! `cli::run`: every command, routed from its parsed arguments onto the
//! ports the composition root wired. Each command finds its project through
//! [`Invocation`], so there is one resolution rule for all of them.

use std::{
    cell::Cell,
    env,
    error::Error,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::exit,
    sync::Arc,
    time::SystemTime,
};

use clap::{CommandFactory, Parser};
use directories::{BaseDirs, ProjectDirs};

use super::{
    add::{add, add_rows},
    args::{
        Args, Command, ConfigCommand, CoordinationCommand, ProcessArgs, ProjectsCommand, RunArgs,
        ServiceCommand, SetArgs, StartArgs, WaitArgs,
    },
    attach::{Announce, attach_named},
    attach_end::AttachEnd,
    cli_exit::CliExit,
    cli_ports::CliPorts,
    completions::{CompletionShell, registration},
    config_command::{set_rows as setting_rows, set_setting, settings, settings_table},
    coordination::coordination,
    doctor::{
        any_failed, completions_probe, database_probe, doctor_report, legacy_probe, project_probe,
        projects_probe,
    },
    emit::{emit, emit_bytes, emit_line, fail_command, fail_framed, print_result, written},
    error::CliError,
    export::{Exported, export, export_rows},
    host_terminal::HostTerminal,
    import::{apply, dry_run_view, import_mode, import_rows, plan, read},
    init::{
        AUTOSTART_ASKED, AUTOSTART_QUESTION, INIT_TITLE, InitView, NOTHING_ADDED, add_chosen,
        added_outro, added_rows, candidate_rows, found_in, nothing_found, offered, suggested,
    },
    invocation::Invocation,
    kv::kv,
    lock::lock,
    note::note,
    output,
    presenter::Presenter,
    process::{follow_logs, named_or_chosen, running_named_or_chosen},
    project_target::{ProjectTarget, resolve_project},
    projects::{REMOVE_ASKED, REMOVE_QUESTION, list, list_rows, removal_rows, remove},
    report::{Report, Row, RowKind, print_error},
    rm::{removable, remove as remove_processes, rm_rows},
    run::RunPlan,
    service_stopped::ServiceStopped,
    set::{set, set_rows},
    todo::todo,
    wait::{condition, wait_text, wait_through},
};
use crate::{
    adapter::{
        client::{
            Action, ActionReport, ClientError, NoticeView, ProjectView, RunningServiceNotifier,
            ServiceClient, SettingView, SettingsView, SettleWatch, UserService, WaitView, process,
        },
        mcp::{self, McpDeps, Scope},
        private_dir::state_dir,
        service::{ProcessAction, ProjectStatus, WaitOutcome, WaitSpec, WaitStatus},
        store::SqliteStore,
    },
    constants::APP_NAME,
    domain::{
        config::ImportMode,
        coordination::Author,
        port::SettingsStore,
        project::Project,
        value::{DetachKey, PositiveDuration, ProcessName, ProcessRef, ProjectRoot},
    },
    error::MusterError,
};

/// The flag that asks a command for JSON output.
const JSON_FLAG: &str = "--json";
/// The flag that answers a confirmation up front.
const YES_FLAG: &str = "--yes";
/// Title of the `note`, `todo`, `kv`, `lock` and `coordination` reports.
const NOTE_TITLE: &str = "muster note";
const TODO_TITLE: &str = "muster todo";
const KV_TITLE: &str = "muster kv";
const LOCK_TITLE: &str = "muster lock";
const COORDINATION_TITLE: &str = "muster coordination";
const COMPLETIONS_TITLE: &str = "muster completions";
/// How `projects rm` ends when the person declined.
const NOTHING_REMOVED: &str = "nothing removed";
/// The command words of `muster projects rm`, for its title.
const PROJECTS_RM_WORDS: &str = "projects rm";
/// Environment variable naming the user's shell, for the completions probe.
const SHELL_ENV: &str = "SHELL";
/// Environment variable controlling the XDG configuration root.
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// XDG configuration root under home when the environment does not override.
const XDG_CONFIG_HOME_DEFAULT: &str = ".config";

/// The command line, parsed. A shell's completion hook is answered here and
/// the process exits; a command line that does not parse exits as a usage
/// error, in JSON when `--json` was asked for.
pub fn parse_args() -> Args {
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    match Args::try_parse() {
        Ok(args) => args,
        Err(error) if error.use_stderr() && json_requested() => {
            emit_line(&output::failure_json(&CliError::Usage(
                error.render().to_string(),
            )));
            exit(CliExit::Usage.code());
        },
        Err(error) => error.exit(),
    }
}

/// Whether the command line asks for JSON output, read before it is parsed
/// so a usage error can be answered in JSON too.
fn json_requested() -> bool {
    env::args_os().any(|arg| arg == JSON_FLAG)
}

/// Runs the command `args` names on `ports`, printing its result and exiting
/// with the code its outcome documents.
pub fn run(args: Args, ports: &dyn CliPorts) {
    let invocation = Invocation::new(args.project, ports);
    let invocation = &invocation;
    match args.command {
        Some(Command::Ps { all, json }) => ps(invocation, all, json),
        Some(Command::Up { json }) => act(
            invocation,
            Presenter::detect(json),
            json,
            Action::Up,
            |_, client, project, watch| Ok(process::up(client, project, watch)?),
        ),
        Some(Command::Down { json }) => act(
            invocation,
            Presenter::detect(json),
            json,
            Action::Down,
            |_, client, project, watch| Ok(process::down(client, project, watch)?),
        ),
        Some(Command::Start(args)) => start(invocation, args, ProcessAction::Start),
        Some(Command::Stop(target)) => control(invocation, target, ProcessAction::Stop),
        Some(Command::Restart(args)) => start(invocation, args, ProcessAction::Restart),
        Some(Command::Wait(args)) => wait(invocation, args),
        Some(Command::Pause(target)) => control(invocation, target, ProcessAction::Pause),
        Some(Command::Resume(target)) => control(invocation, target, ProcessAction::Resume),
        Some(Command::Kill(target)) => control(invocation, target, ProcessAction::Kill),
        Some(Command::Logs {
            name,
            follow,
            screen,
            json,
        }) => logs(invocation, name, follow, screen, json),
        Some(Command::Attach {
            name,
            read_only,
            json,
        }) => attach(invocation, name.as_ref(), read_only, json),
        Some(Command::Notices { json }) => notices(invocation, json),
        Some(Command::Service { command }) => service(command),
        Some(Command::Add(args)) => {
            let json = args.json;
            let presenter = Presenter::detect(json);
            match add(invocation, args) {
                Ok(view) => {
                    print_result(json, &view, |view| written(presenter.show(&add_rows(view))))
                },
                Err(error) => fail_command(json, &error),
            }
        },
        Some(Command::Set(args)) => change(invocation, args),
        Some(Command::Rm { names, json }) => rm(invocation, &names, json),
        Some(Command::Config { command, json }) => config(invocation, command, json),
        Some(Command::Export {
            file,
            force,
            with_env,
            json,
        }) => match export(invocation, file.as_deref(), force, with_env) {
            Ok(Exported::Text(text)) => emit_bytes(text.as_bytes()),
            Ok(Exported::Written(view)) => {
                let presenter = Presenter::detect(json);
                print_result(json, &view, |view| {
                    written(presenter.show(&export_rows(view)))
                });
            },
            Err(error) => fail_command(json, &error),
        },
        Some(Command::Import {
            file,
            merge,
            replace,
            dry_run,
            json,
        }) => import(
            invocation,
            file.as_deref(),
            import_mode(merge, replace),
            dry_run,
            json,
        ),
        Some(Command::Init { yes, json }) => init(invocation, yes, json),
        Some(Command::Run(args)) => run_command(invocation, &args),
        Some(Command::Projects { command, json }) => projects(invocation, command, json),
        Some(Command::Note { command }) => with_root(invocation, |store, root| {
            note(command, store, root, &Author::human()).map(|rows| Report::new(NOTE_TITLE, rows))
        }),
        Some(Command::Todo { command }) => with_root(invocation, |store, root| {
            todo(command, store, root, &Author::human()).map(|rows| Report::new(TODO_TITLE, rows))
        }),
        Some(Command::Kv { command }) => with_root(invocation, |store, root| {
            kv(command, store, root, &Author::human()).map(|rows| Report::new(KV_TITLE, rows))
        }),
        Some(Command::Lock { command }) => with_root(invocation, |store, root| {
            lock(command, store, root).map(|rows| Report::new(LOCK_TITLE, rows))
        }),
        Some(Command::Coordination { command }) => coordinate(invocation, command),
        Some(Command::Mcp {
            print_config,
            agent,
        }) => serve_mcp(invocation, print_config, agent),
        Some(Command::Doctor) => doctor(invocation),
        Some(Command::Completions { shell }) => completions(shell),
        Some(Command::Holder { spec }) => internal(ports.run_holder(&spec)),
        Some(Command::RunService) => internal(ports.run_service()),
        None => bare(invocation),
    }
}

/// Prints `report`, data a command answers with, as plain lines on stdout
/// even at a terminal, so it reads and pastes as it is.
fn show(report: &Report) {
    written(Presenter::Plain.body(report));
}

/// Prints `error` to stderr in the CLI's voice and exits with the code its
/// kind documents: a command's own failure by its kind, stored state that
/// cannot be used as invalid input, anything else as a failed action.
fn fail_with(error: &(dyn Error + 'static)) -> ! {
    print_error(APP_NAME, error);
    let code = if let Some(error) = error.downcast_ref::<CliError>() {
        CliExit::from(error.kind())
    } else if let Some(MusterError::Config(_)) = error.downcast_ref::<MusterError>() {
        CliExit::InvalidInput
    } else {
        CliExit::ActionFailed
    };
    exit(code.code());
}

/// Ends a hidden entry point: quietly on success, in the CLI's voice on
/// failure.
fn internal(result: Result<(), MusterError>) {
    if let Err(error) = result {
        fail_with(&error);
    }
}

/// The registered project the invocation acts on, or the command fails at
/// once, before anything else is done.
fn project_or_fail(invocation: &Invocation<'_>, json: bool) -> Project {
    match invocation
        .store()
        .and_then(|store| invocation.project(&store))
    {
        Ok(project) => project,
        Err(error) => fail_command(json, &error),
    }
}

/// `muster` alone: `ps` inside a project, `ps --all` outside one.
fn bare(invocation: &Invocation<'_>) {
    let inside = invocation
        .store()
        .and_then(|store| invocation.target(&store));
    match inside {
        Ok(ProjectTarget::Registered(_)) => ps(invocation, false, false),
        Ok(ProjectTarget::Unregistered(_)) => ps(invocation, true, false),
        Err(error) => fail_command(false, &error),
    }
}

/// Prints the current project's processes, or every loaded project's with
/// `all`, each under a heading naming the project.
fn ps(invocation: &Invocation<'_>, all: bool, json: bool) {
    let result = (|| {
        let project = if all {
            None
        } else {
            let store = invocation.store()?;
            Some(invocation.project(&store)?)
        };
        let client = connect()?;
        Ok(process::ps(&client, project.as_ref().map(Project::root))?)
    })();
    match result {
        Ok(statuses) => {
            let views: Vec<ProjectView> = statuses.iter().map(ProjectView::from).collect();
            print_result(json, &views, |_| {
                emit_line(&output::render_status(&statuses, all));
            });
        },
        Err(error) => fail_command(json, &error),
    }
}

/// Prints `result` of an action into the frame `presenter` opened and exits
/// with a failure when a process did not end up as asked.
fn finish_action(presenter: Presenter, json: bool, result: Result<ActionReport, CliError>) {
    match result {
        Ok(report) => {
            print_result(json, &report, |report| {
                written(presenter.finish(&output::action_rows(report)));
            });
            if report.failed() {
                exit(CliExit::ActionFailed.code());
            }
        },
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// Runs `run` for `action` on the invocation's project through the service
/// and prints its report through `presenter`: in a frame titled for the
/// action and the project, with a spinner while its processes settle, at a
/// terminal.
fn act(
    invocation: &Invocation<'_>,
    presenter: Presenter,
    json: bool,
    action: Action,
    run: impl FnOnce(
        Presenter,
        &ServiceClient,
        &Project,
        &mut dyn SettleWatch,
    ) -> Result<ActionReport, CliError>,
) {
    let project = project_or_fail(invocation, json);
    written(presenter.open(&output::action_title(action, project.name())));
    let mut spinners = presenter.spinners();
    let result = connect().and_then(|client| run(presenter, &client, &project, &mut spinners));
    drop(spinners);
    finish_action(presenter, json, result);
}

/// This user's service, started or replaced as needed.
///
/// # Errors
/// Returns the [`CliError`] of a service that cannot be reached.
fn connect() -> Result<ServiceClient, CliError> {
    Ok(ServiceClient::connect()?)
}

/// `presenter`, once `check` says it can ask the question a command needs
/// when `needed`; otherwise the command fails at once, before it reaches
/// the service.
fn asking_first(
    presenter: Presenter,
    json: bool,
    needed: bool,
    check: impl FnOnce(Presenter) -> Result<(), CliError>,
) -> Presenter {
    if needed && let Err(error) = check(presenter) {
        fail_command(json, &error);
    }
    presenter
}

/// Applies a lifecycle `action` to the process `target` names, or to the one
/// chosen at a terminal when it names none.
fn control(invocation: &Invocation<'_>, target: ProcessArgs, action: ProcessAction) {
    let ProcessArgs { name, json } = target;
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        name.is_none(),
        Presenter::can_choose_process,
    );
    act(
        invocation,
        presenter,
        json,
        action.into(),
        |presenter, client, project, watch| {
            let pick = named_or_chosen(project, name.as_ref(), presenter);
            process::control(client, project, pick, action, watch)
        },
    );
}

/// Starts or restarts the process `args` names, then, with `--wait`, waits
/// for its new run to be ready.
fn start(invocation: &Invocation<'_>, args: StartArgs, action: ProcessAction) {
    let StartArgs {
        target,
        wait,
        timeout,
    } = args;
    if !wait {
        return control(invocation, target, action);
    }
    let ProcessArgs { name, json } = target;
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        name.is_none(),
        Presenter::can_choose_process,
    );
    let project = project_or_fail(invocation, json);
    let result = connect().and_then(|client| {
        let name = named_or_chosen(&project, name.as_ref(), presenter)(&process::status(
            &client, &project,
        )?)?;
        Ok(wait_through(
            None,
            None,
            timeout.map(PositiveDuration::into_inner),
            |spec| process::control_waiting(&client, &project, &name, action, spec),
            |spec| process::wait(&client, &project, &name, spec),
        )?)
    });
    finish_wait(json, result);
}

/// Waits for what `args` names on its process, chosen at a terminal when it
/// names none.
fn wait(invocation: &Invocation<'_>, args: WaitArgs) {
    let json = args.json;
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        args.name.is_none(),
        Presenter::can_choose_process,
    );
    let until = condition(&args).unwrap_or_else(|error| fail_command(json, &error));
    let project = project_or_fail(invocation, json);
    let result = connect().and_then(|client| {
        let name = named_or_chosen(&project, args.name.as_ref(), presenter)(&process::status(
            &client, &project,
        )?)?;
        let send = |spec: &WaitSpec| process::wait(&client, &project, &name, spec);
        Ok(wait_through(
            until,
            args.since,
            args.timeout.map(PositiveDuration::into_inner),
            send,
            send,
        )?)
    });
    finish_wait(json, result);
}

/// Prints how a wait ended and exits with a failure unless it matched.
fn finish_wait(json: bool, result: Result<WaitOutcome, CliError>) {
    match result {
        Ok(outcome) => {
            print_result(json, &WaitView::from(&outcome), |_| {
                emit_line(&wait_text(&outcome));
            });
            if outcome.status() != WaitStatus::Matched {
                exit(CliExit::ActionFailed.code());
            }
        },
        Err(error) => fail_command(json, &error),
    }
}

/// Prints a process's log or screen, or follows its log until interrupted;
/// the process is chosen at a terminal when `name` names none. A log printed
/// to anything but a terminal is written byte for byte.
fn logs(
    invocation: &Invocation<'_>,
    name: Option<ProcessRef>,
    follow: bool,
    screen: bool,
    json: bool,
) {
    let asking = name.is_none();
    let presenter = asking_first(
        Presenter::detect(json),
        json,
        asking,
        Presenter::can_choose_process,
    );
    let project = project_or_fail(invocation, json);
    if asking {
        written(presenter.open(&output::logs_title(project.name())));
    }
    let frame_open = Cell::new(asking);
    let pick = |status: &ProjectStatus| {
        let chosen = named_or_chosen(&project, name.as_ref(), presenter)(status)?;
        if frame_open.replace(false) {
            written(presenter.conclude(chosen.as_ref()));
        }
        Ok::<_, CliError>(chosen)
    };
    let result = connect().and_then(|client| {
        if follow {
            match follow_logs(&client, &project, pick, json, &mut io::stdout().lock())? {}
        }
        process::logs(&client, &project, pick, screen)
    });
    let framing = if frame_open.get() {
        presenter
    } else {
        Presenter::Plain
    };
    match result {
        Ok(read) if json => print_result(json, &read.text_in(project.root()), |_| {}),
        Ok(read) if !io::stdout().is_terminal() => emit_bytes(read.bytes()),
        Ok(read) => {
            let text = read.text_in(project.root());
            if text.text().is_empty() || text.text().ends_with('\n') {
                emit(text.text());
            } else {
                emit_line(text.text());
            }
        },
        Err(error) => fail_framed(framing, json, &error),
    }
}

/// The key that detaches `muster attach`, from the settings.
///
/// # Errors
/// Returns a [`CliError`] when the settings cannot be read.
fn detach_key(invocation: &Invocation<'_>) -> Result<DetachKey, CliError> {
    Ok(invocation.store()?.load()?.detach_key().clone())
}

/// Says an attach's opening line through `presenter`.
fn announcer(presenter: Presenter) -> Announce {
    Box::new(move |line: &str| presenter.step(line))
}

/// Takes over this terminal to show a running process until the detach key,
/// the process's exit or a signal ends the session; the process is chosen
/// at a terminal, before the session takes it over, when `name` names none.
/// The terminal is checked first, so a command without one starts no
/// service.
fn attach(invocation: &Invocation<'_>, name: Option<&ProcessRef>, read_only: bool, json: bool) {
    let presenter = Presenter::detect(json);
    let prepared = HostTerminal::stdio().and_then(|terminal| {
        if name.is_none() {
            presenter.can_choose_process()?;
        }
        let store = invocation.store()?;
        let project = invocation.project(&store)?;
        Ok((terminal, project, detach_key(invocation)?))
    });
    let (terminal, project, detach) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    written(presenter.open(&output::attach_title(project.name())));
    let result = connect().and_then(|client| {
        let name = running_named_or_chosen(&client, &project, name, presenter)?;
        attach_named(
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

/// Closes an attach's frame with what became of it once the terminal is
/// back, then exits as it ended: a failed exit fails the command, and a
/// signal that ended the session ends this process by the same signal.
fn finish_attach(presenter: Presenter, name: &ProcessName, end: &AttachEnd) {
    if let Some(line) = output::attach_end_line(name, end) {
        written(presenter.conclude(&line));
    }
    match end {
        AttachEnd::Signalled(signal) => {
            let _ = signal.resend();
            exit(CliExit::ActionFailed.code());
        },
        AttachEnd::Exited(outcome) if outcome.is_failure() => {
            exit(CliExit::ActionFailed.code());
        },
        AttachEnd::Detached | AttachEnd::Exited(_) => {},
    }
}

/// Prints the current project's recent notices and the service's.
fn notices(invocation: &Invocation<'_>, json: bool) {
    let project = project_or_fail(invocation, json);
    let result = connect().and_then(|client| Ok(process::notices(&client, &project)?));
    match result {
        Ok(notices) => {
            let views: Vec<NoticeView> = notices.iter().map(NoticeView::from).collect();
            print_result(json, &views, |_| {
                show(&output::notices_rows(&project, &notices, SystemTime::now()));
            });
        },
        Err(error) => fail_command(json, &error),
    }
}

/// Shows or stops the service, never starting it.
fn service(command: ServiceCommand) {
    match command {
        ServiceCommand::Status { json } => match ServiceClient::status() {
            Ok(status) => {
                print_result(json, &status, |status| show(&output::service_rows(status)));
            },
            Err(error) => fail_command(json, &error.into()),
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
                },
                Err(error) => fail_framed(presenter, json, &error.into()),
            }
        },
    }
}

/// `muster set`: the change, then whether a running process waits for its
/// next start to take it.
fn change(invocation: &Invocation<'_>, args: SetArgs) {
    let json = args.json;
    let presenter = Presenter::detect(json);
    match set(invocation, args) {
        Ok(view) => print_result(json, &view, |view| written(presenter.show(&set_rows(view)))),
        Err(error) => fail_command(json, &error),
    }
}

/// `muster rm`: every name is checked before anything is removed, then the
/// removals are stored and the ones that ran waited for.
fn rm(invocation: &Invocation<'_>, names: &[ProcessRef], json: bool) {
    let presenter = Presenter::detect(json);
    let prepared = invocation.store().and_then(|store| {
        let project = invocation.project(&store)?;
        let resolved = removable(&store, &project, names)?;
        Ok((store, project, resolved))
    });
    let (store, project, resolved) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    written(presenter.open(&output::action_title(Action::Remove, project.name())));
    let mut spinners = presenter.spinners();
    let result = connect()
        .and_then(|client| remove_processes(&client, &store, &project, &resolved, &mut spinners));
    drop(spinners);
    match result {
        Ok(view) => {
            print_result(json, &view, |view| {
                written(presenter.finish(&rm_rows(view)))
            });
            if view.failed() {
                exit(CliExit::ActionFailed.code());
            }
        },
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// `muster config`: the list, one value, or a change.
fn config(invocation: &Invocation<'_>, command: Option<ConfigCommand>, json: bool) {
    match command {
        None | Some(ConfigCommand::List) => match settings(invocation) {
            Ok(settings) => print_result(json, &SettingsView::from(&settings), |_| {
                emit_line(&settings_table(&settings));
            }),
            Err(error) => fail_command(json, &error),
        },
        Some(ConfigCommand::Get { key }) => match settings(invocation) {
            Ok(settings) => {
                let view = SettingView::of(&settings, key);
                print_result(json, &view, |view| emit_line(view.value()));
            },
            Err(error) => fail_command(json, &error),
        },
        Some(ConfigCommand::Set { key, value }) => {
            let presenter = Presenter::detect(json);
            match set_setting(invocation, key, &value) {
                Ok(view) => {
                    print_result(json, &view, |view| {
                        written(presenter.show(&setting_rows(view)))
                    });
                },
                Err(error) => fail_command(json, &error),
            }
        },
    }
}

/// `muster import`: the whole file checked, the plan shown or applied in
/// one transaction, and removed processes that ran waited for.
fn import(
    invocation: &Invocation<'_>,
    file: Option<&str>,
    mode: ImportMode,
    dry_run: bool,
    json: bool,
) {
    let presenter = Presenter::detect(json);
    let prepared = invocation.store().and_then(|store| {
        let stack = read(invocation, &store, file)?;
        let planned = plan(&store, &stack, mode)?;
        Ok((store, stack, planned))
    });
    let (store, stack, planned) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    if dry_run {
        let view = dry_run_view(&stack, &planned);
        print_result(json, &view, |view| {
            written(presenter.show(&import_rows(view)))
        });
        return;
    }
    let mut spinners = presenter.spinners();
    let result = if planned.removed().is_empty() {
        apply(&store, &stack, mode, None, &mut spinners)
    } else {
        connect().and_then(|client| apply(&store, &stack, mode, Some(&client), &mut spinners))
    };
    drop(spinners);
    match result {
        Ok(view) => {
            print_result(json, &view, |view| {
                written(presenter.show(&import_rows(view)))
            });
            if view.failed() {
                exit(CliExit::ActionFailed.code());
            }
        },
        Err(error) => fail_command(json, &error),
    }
}

/// `muster init`: what the folder's files offer, chosen at a terminal,
/// listed elsewhere, or the suggested ones added with `--yes`.
fn init(invocation: &Invocation<'_>, yes: bool, json: bool) {
    let presenter = Presenter::detect(json);
    let prepared = invocation.store().and_then(|store| {
        let target = invocation.target(&store)?;
        let candidates = offered(&store, target.root())?;
        Ok((store, target, candidates))
    });
    let (store, target, candidates) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    let root = target.root();
    if json || yes || !presenter.is_interactive() || candidates.is_empty() {
        let added = if yes {
            match add_chosen(&store, root, &suggested(&candidates), None) {
                Ok(added) => added,
                Err(error) => fail_command(json, &error),
            }
        } else {
            Vec::new()
        };
        let view = InitView::builder()
            .candidates(candidates)
            .added(added)
            .build();
        print_result(json, &view, |view| {
            let report = match (view.candidates().is_empty(), yes) {
                (true, _) => nothing_found(),
                (false, true) => added_rows(view.added()),
                (false, false) => candidate_rows(view.candidates()),
            };
            written(presenter.show(&report));
        });
        return;
    }
    written(presenter.open(INIT_TITLE));
    let result = (|| {
        presenter
            .step(&found_in(&candidates))
            .map_err(CliError::Terminal)?;
        let chosen = presenter.choose_candidates(&candidates)?;
        if chosen.is_empty() {
            return Ok(None);
        }
        let autostart = presenter.confirm(
            AUTOSTART_QUESTION,
            *store.load()?.default_autostart(),
            AUTOSTART_ASKED,
            YES_FLAG,
        )?;
        let added = add_chosen(&store, root, &chosen, Some(autostart))?;
        Ok(Some((added, autostart)))
    })();
    match result {
        Ok(None) => written(presenter.conclude(NOTHING_ADDED)),
        Ok(Some((added, autostart))) => {
            written(presenter.conclude(&added_outro(&added, autostart)))
        },
        Err(error) => fail_framed(presenter, false, &error),
    }
}

/// Adds a command to a project, starts it under the service and attaches
/// this terminal to it. Without a terminal, with `--json`, or when the start
/// failed or the command already ended, it reports the start like `start`
/// instead. Its arguments, project and the detach key are checked first and
/// the service reached next, so neither a bad command line nor a service
/// that cannot start leaves anything added.
fn run_command(invocation: &Invocation<'_>, args: &RunArgs) {
    let json = args.json;
    let presenter = Presenter::detect(json);
    let prepared = invocation.store().and_then(|store| {
        let plan = RunPlan::prepare(invocation, &store, args)?;
        let attach = match (json, HostTerminal::stdio()) {
            (false, Ok(terminal)) => Some((terminal, detach_key(invocation)?)),
            (true, _) | (false, Err(_)) => None,
        };
        Ok((store, plan, attach))
    });
    let (store, plan, attach) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    if let Some(name) = plan.project_name() {
        written(presenter.open(&output::action_title(Action::Run, &name)));
    }
    let name = plan.draft().name().clone();
    let mut spinners = presenter.spinners();
    let started = connect().and_then(|client| {
        plan.carry_out(&store, &client, &mut spinners)
            .map(|(project, report)| (client, project, report))
    });
    drop(spinners);
    let (client, project, report) = match started {
        Ok(started) => started,
        Err(error) => fail_framed(presenter, json, &error),
    };
    let Some((terminal, detach)) = attach.filter(|_| !report.failed()) else {
        return finish_action(presenter, json, Ok(report));
    };
    let rows = output::action_rows(&report);
    written(presenter.body(&rows));
    match attach_named(
        client,
        &project,
        name.clone(),
        terminal,
        false,
        detach,
        announcer(presenter),
    ) {
        Ok(end) => finish_attach(presenter, &name, &end),
        Err(CliError::Client(ClientError::NotRunning(_))) => written(presenter.close(&rows)),
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// `muster projects`: the list, or one project stopped and forgotten.
fn projects(invocation: &Invocation<'_>, command: Option<ProjectsCommand>, json: bool) {
    match command {
        None => {
            let listed = invocation
                .store()
                .and_then(|store| list(&store, &invocation.cwd()?));
            match listed {
                Ok(listings) => {
                    print_result(json, &listings, |listings| show(&list_rows(listings)))
                },
                Err(error) => fail_command(json, &error),
            }
        },
        Some(ProjectsCommand::Rm { project, yes }) => {
            remove_project(invocation, &project, yes, json)
        },
    }
}

/// `muster projects rm`: asks at a terminal, needs `--yes` elsewhere, then
/// stops the project's processes and forgets it.
fn remove_project(invocation: &Invocation<'_>, named: &str, yes: bool, json: bool) {
    let presenter = Presenter::detect(json);
    let prepared = invocation.store().and_then(|store| {
        let project =
            resolve_project(Some(named), None, &invocation.cwd()?, &store)?.registered()?;
        if !yes && !presenter.is_interactive() {
            return Err(CliError::NeedsYes(REMOVE_ASKED));
        }
        Ok((store, project))
    });
    let (store, project) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(json, &error),
    };
    written(presenter.open(&output::project_title(PROJECTS_RM_WORDS, project.name())));
    if !yes {
        match presenter.confirm(REMOVE_QUESTION, false, REMOVE_ASKED, YES_FLAG) {
            Ok(true) => {},
            Ok(false) => return written(presenter.conclude(NOTHING_REMOVED)),
            Err(error) => fail_framed(presenter, json, &error),
        }
    }
    let mut spinners = presenter.spinners();
    let result = connect().and_then(|client| remove(&client, &store, &project, &mut spinners));
    drop(spinners);
    match result {
        Ok(removal) => {
            print_result(json, &removal, |removal| {
                written(presenter.finish(&removal_rows(removal)));
            });
            if removal.stopped().failed() {
                exit(CliExit::ActionFailed.code());
            }
        },
        Err(error) => fail_framed(presenter, json, &error),
    }
}

/// Runs a coordination command on the invocation's project, printing its
/// rows as data.
fn with_root(
    invocation: &Invocation<'_>,
    command: impl FnOnce(&SqliteStore, &ProjectRoot) -> Result<Report, CliError>,
) {
    let result = invocation.store().and_then(|store| {
        let project = invocation.project(&store)?;
        command(&store, project.root())
    });
    match result {
        Ok(report) => show(&report),
        Err(error) => fail_command(false, &error),
    }
}

/// `muster coordination`: export or restore the project's shared state.
fn coordinate(invocation: &Invocation<'_>, command: CoordinationCommand) {
    let command = match command {
        CoordinationCommand::Export { file } => CoordinationCommand::Export {
            file: in_cwd_or_fail(invocation, &file),
        },
        CoordinationCommand::Import { file } => CoordinationCommand::Import {
            file: in_cwd_or_fail(invocation, &file),
        },
    };
    with_root(invocation, |store, root| {
        coordination(command, store, root).map(|rows| Report::new(COORDINATION_TITLE, rows))
    });
}

/// `path` read against the current directory, or the command fails.
fn in_cwd_or_fail(invocation: &Invocation<'_>, path: &Path) -> PathBuf {
    invocation
        .in_cwd(path)
        .unwrap_or_else(|error| fail_command(false, &error))
}

/// Serves muster to agents as an MCP server over stdio, blocking until the
/// connecting agent disconnects; with `print_config`, prints the paste-in
/// config snippet, which names no project, instead. Each call resolves its
/// project from the folder `--project` names, or else the current one, so a
/// folder registered later is found and one entry serves every project.
fn serve_mcp(invocation: &Invocation<'_>, print_config: bool, agent: Option<String>) {
    if print_config {
        let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_NAME));
        emit_line(&mcp::connection_config(&executable));
        return;
    }
    let prepared = invocation.store().and_then(|store| {
        let cwd = invocation.cwd()?;
        let base = if invocation.names_project() {
            invocation.target(&store)?.root().as_ref().clone()
        } else {
            cwd.clone()
        };
        Ok((store, Scope::builder().cwd(cwd).base(base).build()))
    });
    let (store, scope) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => fail_command(false, &error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => fail_with(&MusterError::Io(error)),
    };
    let deps = McpDeps::builder()
        .scope(scope)
        .agent(agent)
        .service(Arc::new(UserService))
        .projects(Arc::new(store.clone()))
        .coordination(Arc::new(store.clone()))
        .stacks(Arc::new(store.clone()))
        .settings(Arc::new(store))
        .notifier(Arc::new(RunningServiceNotifier))
        .build();
    if let Err(error) = runtime.block_on(mcp::serve_stdio(deps)) {
        fail_with(&error);
    }
}

/// Runs every doctor probe, prints each line, and exits non-zero if any
/// failed.
fn doctor(invocation: &Invocation<'_>) {
    let opened = invocation.ports().open_state();
    let mut probes = vec![database_probe(&opened)];
    if let Ok(store) = &opened {
        probes.push(projects_probe(store));
        probes.push(project_probe(&invocation.target(store)));
    }
    let config_dir =
        ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.config_dir().to_path_buf());
    probes.push(legacy_probe(state_dir().as_deref(), config_dir.as_deref()));
    let shell = env::var(SHELL_ENV).ok();
    let home = BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default();
    // The XDG root the shells actually read: the override when absolute,
    // else the conventional ~/.config.
    let xdg_config = env::var_os(XDG_CONFIG_HOME_ENV)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(XDG_CONFIG_HOME_DEFAULT));
    probes.push(completions_probe(shell.as_deref(), &home, &xdg_config));
    written(Presenter::detect(false).show(&doctor_report(&probes)));
    if any_failed(&probes) {
        exit(CliExit::ActionFailed.code());
    }
}

/// Prints the shell-specific registration hook lines for completions.
fn completions(shell: CompletionShell) {
    let [hint, line] = registration(shell);
    show(&Report::new(COMPLETIONS_TITLE, vec![
        Row::unlabeled(RowKind::Dim, hint),
        Row::unlabeled(RowKind::Plain, line),
    ]));
}
