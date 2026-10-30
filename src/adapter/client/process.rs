//! The process use cases both drivers run over the service: listing, `up`,
//! `down`, the per-process lifecycle actions, removal, logs and notices.
//! Each one that acts waits for the processes it touched to settle, then
//! reports what became of them along with the notices supervision raised
//! about them meanwhile.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    action::Action,
    action_report::ActionReport,
    client_error::ClientError,
    log_read::LogRead,
    log_slice_view::LogSliceView,
    notice_log::NoticeLog,
    notice_view::NoticeView,
    outcome::Outcome,
    process_outcome::ProcessOutcome,
    process_view::ProcessView,
    project_identity_view::ProjectIdentityView,
    service_access::ServiceAccess,
    service_client::ServiceClient,
    settle_watch::SettleWatch,
    settling::{Settling, UNBOUNDED},
    wait_end::WaitEnd,
};
use crate::{
    adapter::{
        holder::{HolderError, LogQuery, read_log, read_log_stream},
        service::{
            BatchOutcome, EventKind, EventPage, EventSeq, LaunchEnvironment, Notice, ProcessAction,
            ProcessStatus, ProjectStatus, ServiceErrorBody, ServiceErrorKind, ServiceRequest,
            ServiceResponse, WaitOutcome, WaitSpec,
        },
    },
    domain::{
        config::ConfigError,
        port::StackEditLog,
        project::Project,
        value::{ProcessName, ProcessRef, ProjectRoot, UnresolvedProcess},
    },
};

/// Pause between `ps` polls while waiting for processes to settle.
const SETTLE_POLL: Duration = Duration::from_millis(50);
/// How an answer to `ps` of one project that listed none is reported.
const EMPTY_STATUS: &str = "a status listing no project";

/// The processes of the project at `project`, or of every project the
/// service has loaded when it is `None`.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or the
/// project does not load.
pub fn ps(
    client: &ServiceClient,
    project: Option<&ProjectRoot>,
) -> Result<Vec<ProjectStatus>, ClientError> {
    let request = ServiceRequest::Ps {
        project: project.cloned(),
    };
    match client.send(&request)? {
        ServiceResponse::Status(statuses) => Ok(statuses),
        ServiceResponse::Error(body) => Err(refusal(body, None, None)),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)) => Err(ClientError::UnexpectedAnswer((&other).into())),
    }
}

/// Starts `project`'s `autostart` processes that are not running and
/// reports each one the service started, once it settled, and each one it
/// left stopped, with why. `watch` hears what is still starting meanwhile.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn up(
    client: &ServiceClient,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    up_within(client, project, UNBOUNDED, watch)
}

/// Like [`up`], but waits at most `bound` for the processes to settle and
/// otherwise reports them as still starting.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn up_within(
    client: &ServiceClient,
    project: &Project,
    bound: Duration,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    up_sending(client, project, bound, None, watch).map(|(report, _)| report)
}

/// Like [`up_within`], and waits, as `wait` asks, on each process it
/// starts, in the same request; the waits come back in start order.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn up_waiting(
    client: &ServiceClient,
    project: &Project,
    bound: Duration,
    wait: &WaitSpec,
    watch: &mut dyn SettleWatch,
) -> Result<(ActionReport, Vec<WaitOutcome>), ClientError> {
    up_sending(client, project, bound, Some(wait), watch)
}

/// Sends `Up`, with `wait` when given, then reports what settled within
/// `bound` and how each wait ended.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
fn up_sending(
    client: &ServiceClient,
    project: &Project,
    bound: Duration,
    wait: Option<&WaitSpec>,
    watch: &mut dyn SettleWatch,
) -> Result<(ActionReport, Vec<WaitOutcome>), ClientError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let request = ServiceRequest::Up {
        project: project.root().clone(),
        environment: LaunchEnvironment::current(),
        wait: wait.cloned(),
    };
    let response = match wait {
        Some(wait) => client.send_waiting(&request, *wait.timeout())?,
        None => client.send(&request)?,
    };
    let batch = batch(response, project)?;
    let settling = Settling::builder()
        .action(Action::Up)
        .before(&before)
        .names(batch.acted())
        .sent(sent)
        .bound(bound)
        .build();
    let report = report(client, project, settling, watch)?;
    let report = with_found_already(report, &batch, &before);
    let report = with_refusals(report, &batch, client, project)?;
    Ok((report, batch.waits().clone()))
}

/// `report` with each autostart process `batch` found running or starting,
/// stopping, or paused already listed as such, with how `ps` listed it
/// before the request.
fn with_found_already(
    report: ActionReport,
    batch: &BatchOutcome,
    before: &ProjectStatus,
) -> ActionReport {
    let found = [
        (batch.already_running(), Outcome::AlreadyRunning),
        (batch.stopping(), Outcome::StillStopping),
        (batch.paused(), Outcome::AlreadyPaused),
    ];
    let rows: Vec<ProcessOutcome> = found
        .into_iter()
        .flat_map(|(names, outcome)| {
            names.iter().map(move |name| {
                ProcessOutcome::builder()
                    .name(name.clone())
                    .outcome(outcome)
                    .reason(None)
                    .status(find(before, name).map(ProcessView::from))
                    .build()
            })
        })
        .collect();
    report.with_more(rows)
}

/// Lets the caller send its request, unless `watch` says it was interrupted
/// before it was sent.
///
/// # Errors
/// Returns [`ClientError::InterruptedUnsent`] when it was interrupted.
pub fn ready_to_send(watch: &mut dyn SettleWatch) -> Result<(), ClientError> {
    if watch.sending().is_break() {
        return Err(ClientError::InterruptedUnsent);
    }
    Ok(())
}

/// Stops every process of `project` and reports each one the service
/// stopped once it has gone. `watch` hears what is still stopping.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn down(
    client: &ServiceClient,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let batch = batch(
        client.send(&ServiceRequest::Down {
            project: project.root().clone(),
        })?,
        project,
    )?;
    let settling = Settling::builder()
        .action(Action::Down)
        .before(&before)
        .names(batch.acted())
        .sent(sent)
        .build();
    let report = report(client, project, settling, watch)?;
    with_refusals(report, &batch, client, project)
}

/// Applies `action` to the process of `project` that `pick` picks from its
/// current listing, and reports what became of it once it settled, telling
/// `watch` while it waits.
///
/// # Errors
/// Returns the error of `pick` when it finds no process, or a
/// [`ClientError`] when the service cannot be reached or refuses.
pub fn control<E: From<ClientError>>(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, E>,
    action: ProcessAction,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, E> {
    control_within(client, project, pick, action, UNBOUNDED, watch)
}

/// Like [`control`], but waits at most `bound` for the process to settle and
/// otherwise reports it as still starting or stopping. A stop or kill of a
/// process that is not running is not sent, so a crashed process stays
/// crashed and is reported as not running.
///
/// # Errors
/// Returns the error of `pick` when it finds no process, or a
/// [`ClientError`] when the service cannot be reached or refuses.
pub fn control_within<E: From<ClientError>>(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, E>,
    action: ProcessAction,
    bound: Duration,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, E> {
    let before = status(client, project)?;
    let name = pick(&before)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let idle = find(&before, &name).is_some_and(|process| !process.state().may_be_running());
    if !(Action::from(action).halts() && idle) {
        send_control(client, project, &name, action)?;
    }
    let names = [name];
    let settling = Settling::builder()
        .action(action.into())
        .before(&before)
        .names(&names)
        .sent(sent)
        .bound(bound)
        .build();
    Ok(report(client, project, settling, watch)?)
}

/// Starts `name`, which `muster run` has just added to `project`, and
/// reports it like `start`, telling `watch` while it waits.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn start_added(
    client: &ServiceClient,
    project: &Project,
    name: ProcessName,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    let before = status(client, project)?;
    let sent = SystemTime::now();
    send_control(client, project, &name, ProcessAction::Start)?;
    let names = [name];
    let settling = Settling::builder()
        .action(Action::Run)
        .before(&before)
        .names(&names)
        .sent(sent)
        .build();
    report(client, project, settling, watch)
}

/// Sends `action` for the process `name` of `project`, with the caller's
/// environment for a launch.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
fn send_control(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessName,
    action: ProcessAction,
) -> Result<(), ClientError> {
    expect_ok(
        client.send(&ServiceRequest::Control {
            project: project.root().clone(),
            name: name.clone(),
            action,
            environment: LaunchEnvironment::current(),
            wait: None,
        })?,
        project,
        Some(name),
    )
}

/// Applies `action` to the process `name` of `project` and waits, as `wait`
/// asks, on the run the action leaves; a start or restart only counts its
/// new run.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn control_waiting(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessName,
    action: ProcessAction,
    wait: &WaitSpec,
) -> Result<WaitOutcome, ClientError> {
    let request = ServiceRequest::Control {
        project: project.root().clone(),
        name: name.clone(),
        action,
        environment: LaunchEnvironment::current(),
        wait: Some(wait.clone()),
    };
    waited(
        client.send_waiting(&request, *wait.timeout())?,
        project,
        name,
    )
}

/// Waits, as `wait` asks, for the process `name` of `project`.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn wait(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessName,
    wait: &WaitSpec,
) -> Result<WaitOutcome, ClientError> {
    let request = ServiceRequest::Wait {
        project: project.root().clone(),
        name: name.clone(),
        wait: wait.clone(),
    };
    waited(
        client.send_waiting(&request, *wait.timeout())?,
        project,
        name,
    )
}

/// The lifecycle events of `project` after `since` (the newest few without
/// it), of `kinds` (every kind when empty), waiting at most `wait` for one
/// when there are none yet.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn events(
    client: &ServiceClient,
    project: &Project,
    since: Option<EventSeq>,
    kinds: &[EventKind],
    wait: Duration,
) -> Result<EventPage, ClientError> {
    let request = ServiceRequest::Events {
        project: project.root().clone(),
        since,
        kinds: kinds.to_vec(),
        wait,
    };
    match client.send_waiting(&request, wait)? {
        ServiceResponse::Events(page) => Ok(page),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, None)),
    }
}

/// How the wait on the process `name` of `project` ended, from `response`.
///
/// # Errors
/// Returns the [`ClientError`] for a refusal or an unexpected answer.
fn waited(
    response: ServiceResponse,
    project: &Project,
    name: &ProcessName,
) -> Result<WaitOutcome, ClientError> {
    match response {
        ServiceResponse::Waited(outcome) => Ok(outcome),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, Some(name))),
    }
}

/// Waits at most `bound` for the processes `names` of `project`, removed
/// from its stack at `sent` while listed as `before`, to stop and leave the
/// listing, and reports each as stopped, still stopping, or as not having
/// run. `watch` hears what is still stopping.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or the wait
/// is interrupted.
pub fn settle_removed(
    client: &ServiceClient,
    project: &Project,
    before: &ProjectStatus,
    names: &[ProcessName],
    sent: SystemTime,
    bound: Duration,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    let settling = Settling::builder()
        .action(Action::Remove)
        .before(before)
        .names(names)
        .sent(sent)
        .bound(bound)
        .build();
    report(client, project, settling, watch)
}

/// Whether a service that already runs, reached through `service`, has
/// `project` loaded and lists `name` as possibly running. It never starts
/// the service, and an answer it cannot get reads as not running, since this
/// only tells a caller whether an edit it already made waits for a restart.
pub fn runs_now(service: &dyn ServiceAccess, project: &ProjectRoot, name: &ProcessName) -> bool {
    service
        .loaded_status(project)
        .ok()
        .flatten()
        .is_some_and(|status| {
            status.processes().iter().any(|process| {
                process.name().key() == name.key() && process.state().may_be_running()
            })
        })
}

/// The recent notices of `project` and of the service, with the agent edits
/// `edits` keeps for the project, oldest first. An edit shows whether or not
/// a service ran when it was made.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses,
/// or the edit log cannot be read.
pub fn notices(
    client: &ServiceClient,
    edits: &dyn StackEditLog,
    project: &Project,
) -> Result<Vec<Notice>, ClientError> {
    with_edits(service_notices(client, project)?, edits, project)
}

/// The agent edits `edits` keeps for `project`, with the recent notices of
/// `project` and of the service when one already runs, oldest first. The
/// service is never started.
///
/// # Errors
/// Returns a [`ClientError`] when a running service does not answer, or the
/// edit log cannot be read.
pub fn stored_notices(
    service: &dyn ServiceAccess,
    edits: &dyn StackEditLog,
    project: &Project,
) -> Result<NoticeLog, ClientError> {
    let live = service.running_notices(project.root())?;
    Ok(NoticeLog::builder()
        .service_running(live.is_some())
        .notices(with_edits(live.unwrap_or_default(), edits, project)?)
        .build())
}

/// `notices` with the agent edits `edits` keeps for `project`, oldest first.
///
/// # Errors
/// Returns a [`ClientError`] when the edit log cannot be read.
fn with_edits(
    mut notices: Vec<Notice>,
    edits: &dyn StackEditLog,
    project: &Project,
) -> Result<Vec<Notice>, ClientError> {
    notices.extend(
        edits
            .edits(project.root())?
            .iter()
            .map(|edit| Notice::for_edit(project.root(), edit)),
    );
    notices.sort_by_key(|notice| *notice.at());
    Ok(notices)
}

/// The recent notices the service raised about `project` and itself, oldest
/// first.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
fn service_notices(client: &ServiceClient, project: &Project) -> Result<Vec<Notice>, ClientError> {
    match client.send(&ServiceRequest::Notices {
        project: project.root().clone(),
    })? {
        ServiceResponse::Notices(notices) => Ok(notices),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, None)),
    }
}

/// The log of the process `pick` picks from `project`'s listing, the older
/// generation first, whether or not it runs, or with `screen` its live
/// terminal screen.
///
/// # Errors
/// Returns the error of `pick` when it finds no process, or a
/// [`ClientError`] when the service cannot be reached or the log cannot be
/// read.
pub fn logs<E: From<ClientError>>(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, E>,
    screen: bool,
) -> Result<LogRead, E> {
    let name = pick(&status(client, project)?)?;
    let read = LogRead::builder().name(name.clone());
    if screen {
        return match client.send(&ServiceRequest::Screen {
            project: project.root().clone(),
            name: name.clone(),
        })? {
            ServiceResponse::Screen(screen) => {
                Ok(read.log(None).end(None).bytes(screen.into_bytes()).build())
            },
            other @ (ServiceResponse::Hello { .. }
            | ServiceResponse::Ok
            | ServiceResponse::Batch(_)
            | ServiceResponse::Log(_)
            | ServiceResponse::Status(_)
            | ServiceResponse::Located(_)
            | ServiceResponse::Notices(_)
            | ServiceResponse::Waited(_)
            | ServiceResponse::Events(_)
            | ServiceResponse::Error(_)) => Err(unexpected(other, project, Some(&name)).into()),
        };
    }
    let log = log_path(client, project, &name)?;
    let (start, bytes) = read_log_stream(&log).map_err(|error| log_read_error(&log, error))?;
    Ok(read
        .log(Some(log))
        .end(Some(start.advanced(bytes.len() as u64)))
        .bytes(bytes)
        .build())
}

/// The failure of reading the log at `path`.
pub fn log_error(path: &Path, source: io::Error) -> ClientError {
    ClientError::Log {
        path: path.to_path_buf(),
        source,
    }
}

/// The failure the log reader met reading the log at `path`: a cursor past
/// its end is the caller's, anything else a failure reading it.
pub fn log_read_error(path: &Path, error: HolderError) -> ClientError {
    match error {
        HolderError::CursorAhead { cursor, end } => ClientError::CursorAhead { cursor, end },
        HolderError::Io(source) => log_error(path, source),
        other @ (HolderError::Pty(_)
        | HolderError::Json(_)
        | HolderError::UnexpectedFrame(_)
        | HolderError::MalformedPayload(_)
        | HolderError::MissingProject
        | HolderError::MissingChildPid
        | HolderError::UnrecordablePid(_)
        | HolderError::MissingChildInput
        | HolderError::InsecureSocketDir(_)
        | HolderError::NotReady(_)
        | HolderError::NoSnapshot(_)
        | HolderError::FailedToStart { .. }) => {
            log_error(path, io::Error::other(other.to_string()))
        },
    }
}

/// Reads what `query` asks of the log of the process of `project` that
/// `pick` chooses from its listing, stripped and filtered, with where its
/// current run began.
///
/// # Errors
/// Returns the error `pick` gives, or a [`ClientError`] when the service
/// cannot be reached, the cursor is past the end of the output, or the log
/// cannot be read.
pub fn log_slice<E: From<ClientError>>(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, E>,
    query: &LogQuery,
) -> Result<LogSliceView, E> {
    let status = status(client, project)?;
    let name = pick(&status)?;
    let run_start = find(&status, &name).and_then(|process| *process.run_start());
    let log = log_path(client, project, &name)?;
    let slice = read_log(&log, query).map_err(|error| log_read_error(&log, error))?;
    Ok(LogSliceView::for_slice(name, &slice, run_start))
}

/// The log every run of the process `name` writes to.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or does not
/// know the process.
pub fn log_path(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessName,
) -> Result<PathBuf, ClientError> {
    match client.send(&ServiceRequest::Log {
        project: project.root().clone(),
        name: name.clone(),
    })? {
        ServiceResponse::Log(log) => Ok(log),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, Some(name))),
    }
}

/// The current status of `project`.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or the
/// project does not load.
pub fn status(client: &ServiceClient, project: &Project) -> Result<ProjectStatus, ClientError> {
    match client.send(&ServiceRequest::Ps {
        project: Some(project.root().clone()),
    })? {
        ServiceResponse::Status(statuses) => statuses
            .into_iter()
            .next()
            .ok_or(ClientError::UnexpectedAnswer(EMPTY_STATUS)),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, None)),
    }
}

/// Picks, from a listing of `project`, the process `name` means.
pub fn named<'a, E: From<ClientError>>(
    project: &'a Project,
    name: &'a ProcessRef,
) -> impl FnOnce(&ProjectStatus) -> Result<ProcessName, E> + 'a {
    move |status| Ok(resolve(status, project, name)?)
}

/// The process of `status` that `name` means.
///
/// # Errors
/// Returns [`ClientError::UnknownProcess`] listing every valid name, or
/// [`ClientError::AmbiguousProcess`].
pub fn resolve(
    status: &ProjectStatus,
    project: &Project,
    name: &ProcessRef,
) -> Result<ProcessName, ClientError> {
    let names: Vec<ProcessName> = status
        .processes()
        .iter()
        .map(|process| process.name().clone())
        .collect();
    resolve_among(names, project, name)
}

/// The one of `names`, the processes of `project`, that `name` means.
///
/// # Errors
/// Returns [`ClientError::UnknownProcess`] listing every valid name, or
/// [`ClientError::AmbiguousProcess`].
pub fn resolve_among(
    names: Vec<ProcessName>,
    project: &Project,
    name: &ProcessRef,
) -> Result<ProcessName, ClientError> {
    match name.resolve(&names) {
        Ok(found) => Ok(found.clone()),
        Err(UnresolvedProcess::Unknown(typed)) => Err(ClientError::UnknownProcess {
            name: typed.to_string(),
            project: project.name().clone(),
            valid: names,
        }),
        Err(UnresolvedProcess::Ambiguous(typed, candidates)) => {
            Err(ClientError::AmbiguousProcess {
                name: typed.to_string(),
                project: project.name().clone(),
                candidates,
            })
        },
    }
}

/// Waits for each process `settling` names to settle, bounded by its action's
/// wait for the longest stop grace among them and by its own bound, then
/// reports what became of each against how it was listed before. Notices
/// raised since the request about those processes belong to the report; any
/// other notices are kept apart as context. `watch` hears which are still on
/// their way at each look, and may end the wait.
///
/// # Errors
/// Returns [`ClientError::Interrupted`] when `watch` ended the wait, or
/// another [`ClientError`] when the service cannot be reached.
fn report(
    client: &ServiceClient,
    project: &Project,
    settling: Settling<'_>,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, ClientError> {
    let (action, before, names, sent, bound) = (
        settling.action(),
        settling.before(),
        settling.names(),
        settling.sent(),
        settling.bound(),
    );
    let stop_grace = names
        .iter()
        .filter_map(|name| find(before, name).map(|process| *process.stop_grace()))
        .max()
        .unwrap_or_default();
    let own = action.settle_timeout(stop_grace);
    let deadline = Instant::now() + own.min(bound);
    let mut moved: HashSet<ProcessName> = HashSet::new();
    let (after, wait_ended) = loop {
        let after = status(client, project)?;
        let moving: Vec<&ProcessStatus> = names
            .iter()
            .filter_map(|name| find(&after, name))
            .filter(|process| action.settling(process))
            .collect();
        moved.extend(moving.iter().map(|process| process.name().clone()));
        if watch.waiting(action, &moving).is_break() {
            return Err(ClientError::Interrupted);
        }
        if moving.is_empty() {
            break (after, WaitEnd::Settled);
        }
        if Instant::now() >= deadline {
            break (after, wait_end_at_deadline(own, bound));
        }
        thread::sleep(SETTLE_POLL);
    };
    if watch.waiting(action, &[]).is_break() {
        return Err(ClientError::Interrupted);
    }
    let processes = names
        .iter()
        .map(|name| {
            let now = find(&after, name);
            ProcessOutcome::builder()
                .name(name.clone())
                .outcome(Outcome::settle(
                    action,
                    find(before, name),
                    now,
                    moved.contains(name),
                ))
                .reason(
                    now.and_then(|status| status.blocked_on().as_ref())
                        .filter(|_| !action.halts())
                        .map(ToString::to_string),
                )
                .status(now.map(ProcessView::from))
                .build()
        })
        .collect();
    let (notices, other_notices): (Vec<NoticeView>, Vec<NoticeView>) =
        service_notices(client, project)?
            .iter()
            .filter(|notice| *notice.at() >= sent)
            .map(|notice| NoticeView::for_project(notice, project))
            .partition(|notice| {
                notice
                    .process()
                    .as_ref()
                    .is_some_and(|process| names.contains(process))
            });
    Ok(ActionReport::builder()
        .project(ProjectIdentityView::from(project))
        .action(action)
        .processes(processes)
        .notices(notices)
        .other_notices(other_notices)
        .wait_ended(wait_ended)
        .build())
}

/// How a wait that reached its deadline ended: cut short when the caller's
/// `bound` was shorter than the action's `own` timeout, timed out otherwise.
fn wait_end_at_deadline(own: Duration, bound: Duration) -> WaitEnd {
    if bound < own {
        WaitEnd::CutShort
    } else {
        WaitEnd::TimedOut
    }
}

/// `report` with each process `batch` left alone listed as refused, with
/// its reason and how `ps` lists it now, and the notices about those
/// processes counted as the report's own rather than as context.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached.
fn with_refusals(
    report: ActionReport,
    batch: &BatchOutcome,
    client: &ServiceClient,
    project: &Project,
) -> Result<ActionReport, ClientError> {
    if batch.refused().is_empty() {
        return Ok(report);
    }
    let now = status(client, project)?;
    let refused: Vec<ProcessOutcome> = batch
        .refused()
        .iter()
        .map(|refusal| {
            ProcessOutcome::builder()
                .name(refusal.name().clone())
                .outcome(Outcome::Refused)
                .reason(Some(refusal.reason().clone()))
                .status(find(&now, refusal.name()).map(ProcessView::from))
                .build()
        })
        .collect();
    let names: Vec<ProcessName> = batch
        .refused()
        .iter()
        .map(|refusal| refusal.name().clone())
        .collect();
    Ok(report.with_more(refused).claiming_notices_about(&names))
}

/// What a project-wide request did, from its answer.
///
/// # Errors
/// Returns the [`ClientError`] for a refusal or an unexpected answer.
fn batch(response: ServiceResponse, project: &Project) -> Result<BatchOutcome, ClientError> {
    match response {
        ServiceResponse::Batch(batch) => Ok(batch),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, None)),
    }
}

/// The process `name` of `status`.
fn find<'a>(status: &'a ProjectStatus, name: &ProcessName) -> Option<&'a ProcessStatus> {
    status
        .processes()
        .iter()
        .find(|process| process.name() == name)
}

/// `Ok` for a carried-out request about `project`, and the process `name`
/// when it names one; the refusal otherwise.
///
/// # Errors
/// Returns the [`ClientError`] for a refusal or an unexpected answer.
pub fn expect_ok(
    response: ServiceResponse,
    project: &Project,
    name: Option<&ProcessName>,
) -> Result<(), ClientError> {
    match response {
        ServiceResponse::Ok => Ok(()),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)
        | ServiceResponse::Error(_)) => Err(unexpected(other, project, name)),
    }
}

/// The failure for `response`, which the caller did not expect: the refusal
/// it carries, or an answer the caller cannot use.
pub fn unexpected(
    response: ServiceResponse,
    project: &Project,
    name: Option<&ProcessName>,
) -> ClientError {
    match response {
        ServiceResponse::Error(body) => refusal(body, Some(project), name),
        other @ (ServiceResponse::Hello { .. }
        | ServiceResponse::Ok
        | ServiceResponse::Batch(_)
        | ServiceResponse::Log(_)
        | ServiceResponse::Status(_)
        | ServiceResponse::Located(_)
        | ServiceResponse::Screen(_)
        | ServiceResponse::Notices(_)
        | ServiceResponse::Waited(_)
        | ServiceResponse::Events(_)) => ClientError::UnexpectedAnswer((&other).into()),
    }
}

/// The failure for the service's refusal `body` of a request about
/// `project` and the process `name`.
fn refusal(
    body: ServiceErrorBody,
    project: Option<&Project>,
    name: Option<&ProcessName>,
) -> ClientError {
    match (body.kind(), project, name) {
        (ServiceErrorKind::UnknownProject, Some(project), _) => {
            ClientError::Config(ConfigError::UnknownProject {
                root: project.root().as_ref().clone(),
            })
        },
        (ServiceErrorKind::UnknownProcess, Some(project), Some(name)) => {
            ClientError::UnknownProcess {
                name: name.to_string(),
                project: project.name().clone(),
                valid: body.candidates().clone(),
            }
        },
        (ServiceErrorKind::NotRunning, _, Some(name)) => ClientError::NotRunning(name.clone()),
        (kind, ..) => ClientError::Refused {
            kind: *kind,
            message: body.message().clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            client::fake_service::FakeService,
            service::NoticeLevel,
            store::{
                SqliteStore,
                test_support::{fresh_store, spec},
            },
        },
        domain::{config::StackEdit, coordination::Author, port::StackStore},
    };

    /// A wait whose caller's bound is the shorter one was cut short; one that
    /// ran the action's own timeout timed out.
    #[test]
    fn a_wait_cut_by_a_shorter_bound_is_cut_short() {
        let own = Duration::from_secs(10);

        assert_eq!(
            wait_end_at_deadline(own, Duration::from_secs(5)),
            WaitEnd::CutShort
        );
        assert_eq!(wait_end_at_deadline(own, UNBOUNDED), WaitEnd::TimedOut);
        assert_eq!(wait_end_at_deadline(own, own), WaitEnd::TimedOut);
    }

    /// A store whose `shop` project had its `web` process removed by an
    /// agent, the edit that removal recorded, and the project.
    fn edited_project() -> (SqliteStore, TempDir, Project, StackEdit) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir).unwrap();
        let registration = store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        let name = ProcessName::try_new("web").unwrap();
        let agent = Author::agent("claude");
        store.remove_process(&root, &name, &agent).unwrap();
        let edit = StackEdit::Removed {
            author: agent,
            name,
        };
        (store, scratch, registration.project().clone(), edit)
    }

    /// A notice raised long before any edit.
    fn earlier() -> Notice {
        Notice::builder()
            .at(SystemTime::UNIX_EPOCH)
            .project(None)
            .process(None)
            .level(NoticeLevel::Error)
            .message("old".to_string())
            .build()
    }

    /// An agent edit recorded while no service ran is shown among the
    /// service's notices, in time order, as a warning on its process.
    #[test]
    fn a_stored_edit_joins_the_service_notices_in_time_order() {
        let (store, _scratch, project, edit) = edited_project();

        let notices = with_edits(vec![earlier()], &store, &project).unwrap();

        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0], earlier());
        assert_eq!(notices[1].message(), &edit.to_string());
        assert_eq!(notices[1].project().as_ref(), Some(project.root()));
        assert_eq!(
            notices[1].process().as_ref().map(AsRef::as_ref),
            Some("web")
        );
        assert_eq!(*notices[1].level(), NoticeLevel::Warning);
    }

    /// The stored notices are read with no service running and none started,
    /// and a running service's own join them.
    #[test]
    fn stored_notices_never_start_the_service() {
        let (store, _scratch, project, edit) = edited_project();

        let stopped = FakeService::default();
        let running = FakeService::running(vec![earlier()]);
        let idle = stored_notices(&stopped, &store, &project).unwrap();
        let live = stored_notices(&running, &store, &project).unwrap();

        assert_eq!(stopped.connects() + running.connects(), 0);
        assert!(!idle.service_running());
        let idle_messages: Vec<&String> = idle.notices().iter().map(Notice::message).collect();
        assert_eq!(idle_messages, [&edit.to_string()]);
        assert!(live.service_running());
        assert_eq!(live.notices().len(), 2);
        assert_eq!(live.notices()[0], earlier());
    }
}
