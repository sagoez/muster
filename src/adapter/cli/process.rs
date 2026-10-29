//! Process commands over the service: `ps`, `up`, `down`, the per-process
//! lifecycle actions, `logs` and `notices`. Each one that acts waits for the
//! processes it touched to settle, then reports what became of them along
//! with the notices supervision raised about them meanwhile.

use std::{
    collections::HashSet,
    convert::Infallible,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    action::Action,
    action_report::ActionReport,
    error::CliError,
    json_log_lines::JsonLogLines,
    log_follower::LogFollower,
    log_text::LogText,
    notice_view::NoticeView,
    outcome::Outcome,
    presenter::Presenter,
    process_outcome::ProcessOutcome,
    process_view::ProcessView,
    service_client::ServiceClient,
    settle_watch::SettleWatch,
    settling::{Settling, UNBOUNDED},
    wait_end::WaitEnd,
};
use crate::{
    adapter::{
        holder::rotated_path,
        service::{
            BatchOutcome, Notice, NoticeLevel, ProcessAction, ProcessStatus, ProjectStatus,
            ServiceErrorBody, ServiceErrorKind, ServiceRequest, ServiceResponse,
        },
    },
    domain::{
        process::{ProcessKind, ProcessState},
        value::{ProcessName, ProcessRef, UnresolvedProcess},
    },
};

/// Pause between `ps` polls while waiting for processes to settle.
const SETTLE_POLL: Duration = Duration::from_millis(50);
/// Pause between reads of a followed log.
const FOLLOW_POLL: Duration = Duration::from_millis(200);
/// Longest `muster run` waits for the service to see the command it added
/// to `muster.yml`, which reaches the service through its config watcher.
const PICKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// How an answer to `ps` of one project that listed none is reported.
const EMPTY_STATUS: &str = "a status listing no project";

/// What `ps` reports: the processes of `project`, or of every project the
/// service has loaded when it is `None`.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or the project
/// does not load.
pub fn ps(client: &ServiceClient, project: Option<&Path>) -> Result<Vec<ProjectStatus>, CliError> {
    let request = ServiceRequest::Ps {
        project: project.map(Path::to_path_buf),
    };
    match client.send(&request)? {
        ServiceResponse::Status(statuses) => Ok(statuses),
        other => Err(unexpected(other, project.unwrap_or(Path::new("")), None)),
    }
}

/// Starts `project`'s `autostart` processes that are not running and
/// reports each one the service started, once it settled, and each one it
/// left stopped, with why. `watch` hears what is still starting meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn up(
    client: &ServiceClient,
    project: &Path,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let batch = batch(
        client.send(&ServiceRequest::Up {
            project: project.to_path_buf(),
        })?,
        project,
    )?;
    let settling = Settling::builder()
        .action(Action::Up)
        .before(&before)
        .names(batch.acted())
        .sent(sent)
        .build();
    let report = report(client, project, settling, watch)?;
    with_refusals(report, &batch, client, project)
}

/// Lets the command send its request, unless `watch` says the person
/// interrupted it before it was sent.
///
/// # Errors
/// Returns [`CliError::InterruptedUnsent`] when it was interrupted.
pub(super) fn ready_to_send(watch: &mut dyn SettleWatch) -> Result<(), CliError> {
    if watch.sending().is_break() {
        return Err(CliError::InterruptedUnsent);
    }
    Ok(())
}

/// Whether `process` is an agent that may still be running.
fn running_agent(process: &ProcessStatus) -> bool {
    *process.kind() == ProcessKind::Agent && process.state().may_be_running()
}

/// Whether any agent of `project` may still be running, so stopping its
/// agents would stop something.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or the project
/// does not load.
pub fn agents_may_run(client: &ServiceClient, project: &Path) -> Result<bool, CliError> {
    Ok(status(client, project)?
        .processes()
        .iter()
        .any(running_agent))
}

/// Stops `project`'s commands and terminals, and its agents too when
/// `agents` is set, and reports each one the service stopped once it has
/// gone, and each agent left running. `watch` hears what is still stopping.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn down(
    client: &ServiceClient,
    project: &Path,
    agents: bool,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let batch = batch(
        client.send(&ServiceRequest::Down {
            project: project.to_path_buf(),
            agents,
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
    let report = with_refusals(report, &batch, client, project)?;
    if agents {
        return Ok(report);
    }
    let kept: Vec<ProcessOutcome> = status(client, project)?
        .processes()
        .iter()
        .filter(|process| running_agent(process))
        .map(|process| {
            ProcessOutcome::builder()
                .name(process.name().clone())
                .outcome(Outcome::KeptRunning)
                .status(Some(ProcessView::from(process)))
                .build()
        })
        .collect();
    Ok(report.with_more(kept))
}

/// Applies `action` to the process of `project` that `pick` picks from its
/// current listing, and reports what became of it once it settled, telling
/// `watch` while it waits.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the service
/// cannot be reached or refuses.
pub fn control(
    client: &ServiceClient,
    project: &Path,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    action: ProcessAction,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    control_within(client, project, pick, action, UNBOUNDED, watch)
}

/// Like [`control`], but waits at most `bound` for the process to settle and
/// otherwise reports it as still starting or stopping.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the service
/// cannot be reached or refuses.
pub fn control_within(
    client: &ServiceClient,
    project: &Path,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    action: ProcessAction,
    bound: Duration,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    let name = pick(&before)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    expect_ok(
        client.send(&ServiceRequest::Control {
            project: project.to_path_buf(),
            name: name.clone(),
            action,
        })?,
        project,
        Some(&name),
    )?;
    let names = [name];
    let settling = Settling::builder()
        .action(action.into())
        .before(&before)
        .names(&names)
        .sent(sent)
        .bound(bound)
        .build();
    report(client, project, settling, watch)
}

/// Starts `name`, which `muster run` has just added to `project`'s config,
/// once the service has picked the change up, and reports it like `start`,
/// telling `watch` while it waits.
///
/// # Errors
/// Returns [`CliError::NotPickedUp`] when the service does not know the
/// process within a few seconds, or another [`CliError`] when it cannot be
/// reached or refuses.
pub fn start_registered(
    client: &ServiceClient,
    project: &Path,
    name: ProcessName,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let deadline = Instant::now() + PICKUP_TIMEOUT;
    let request = ServiceRequest::Control {
        project: project.to_path_buf(),
        name: name.clone(),
        action: ProcessAction::Start,
    };
    loop {
        let before = status(client, project)?;
        let sent = SystemTime::now();
        match client.send(&request)? {
            ServiceResponse::Ok => {
                let names = [name];
                let settling = Settling::builder()
                    .action(Action::Run)
                    .before(&before)
                    .names(&names)
                    .sent(sent)
                    .build();
                return report(client, project, settling, watch);
            },
            ServiceResponse::Error(body)
                if *body.kind() == ServiceErrorKind::UnknownProcess
                    && Instant::now() < deadline =>
            {
                thread::sleep(SETTLE_POLL);
            },
            ServiceResponse::Error(body) if *body.kind() == ServiceErrorKind::UnknownProcess => {
                return Err(CliError::NotPickedUp {
                    name,
                    project: project.to_path_buf(),
                    waited: PICKUP_TIMEOUT,
                });
            },
            other => return Err(unexpected(other, project, Some(&name))),
        }
    }
}

/// The recent notices of `project` and of the service, oldest first.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn notices(client: &ServiceClient, project: &Path) -> Result<Vec<Notice>, CliError> {
    match client.send(&ServiceRequest::Notices {
        project: project.to_path_buf(),
    })? {
        ServiceResponse::Notices(notices) => Ok(notices),
        other => Err(unexpected(other, project, None)),
    }
}

/// What `muster logs` shows of the process `pick` picks from `project`'s
/// listing: its log, the older generation first, whether or not it runs, or
/// with `screen` its live terminal screen.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process or the log cannot be
/// read.
pub fn logs(
    client: &ServiceClient,
    project: &Path,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    screen: bool,
) -> Result<LogText, CliError> {
    let name = pick(&status(client, project)?)?;
    let text = LogText::builder()
        .project(project.to_path_buf())
        .name(name.clone());
    if screen {
        return match client.send(&ServiceRequest::Screen {
            project: project.to_path_buf(),
            name: name.clone(),
        })? {
            ServiceResponse::Screen(screen) => Ok(text.log(None).text(screen).build()),
            other => Err(unexpected(other, project, Some(&name))),
        };
    }
    let log = log_path(client, project, &name)?;
    let mut bytes = Vec::new();
    for path in [rotated_path(&log), log.clone()] {
        match fs::read(&path) {
            Ok(read) => bytes.extend(read),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(source) => return Err(CliError::Log { path, source }),
        }
    }
    Ok(text
        .log(Some(log))
        .text(String::from_utf8_lossy(&bytes).into_owned())
        .build())
}

/// Copies the log of the process `pick` picks into `out`, the older
/// generation first, then follows it across rotations and restarts until the
/// command is interrupted, waiting for a process that is not running to
/// write. With `json`, each log line is written as one JSON [`LogText`] line.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the log cannot be
/// read or written out.
pub fn follow_logs(
    client: &ServiceClient,
    project: &Path,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    json: bool,
    out: &mut dyn Write,
) -> Result<Infallible, CliError> {
    let name = pick(&status(client, project)?)?;
    let log = log_path(client, project, &name)?;
    if !json {
        return follow(&log, out);
    }
    let template = LogText::builder()
        .project(project.to_path_buf())
        .name(name)
        .log(Some(log.clone()))
        .text(String::new())
        .build();
    follow(&log, &mut JsonLogLines::new(out, template))
}

/// Copies `log`'s older generation and then the live log into `out`,
/// following the live one until the command is interrupted.
///
/// # Errors
/// Returns a [`CliError`] when the log cannot be read or written out.
fn follow(log: &Path, out: &mut dyn Write) -> Result<Infallible, CliError> {
    let rotated = rotated_path(log);
    match fs::read(&rotated) {
        Ok(bytes) => out.write_all(&bytes).map_err(CliError::Output)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(source) => {
            return Err(CliError::Log {
                path: rotated,
                source,
            });
        },
    }
    let mut follower = LogFollower::new(log.to_path_buf());
    loop {
        follower.pump(out).map_err(|source| match source.kind() {
            io::ErrorKind::BrokenPipe => CliError::Output(source),
            _ => CliError::Log {
                path: log.to_path_buf(),
                source,
            },
        })?;
        out.flush().map_err(CliError::Output)?;
        thread::sleep(FOLLOW_POLL);
    }
}

/// The log every run of the process `name` writes to.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or does not
/// know the process.
fn log_path(
    client: &ServiceClient,
    project: &Path,
    name: &ProcessName,
) -> Result<PathBuf, CliError> {
    match client.send(&ServiceRequest::Log {
        project: project.to_path_buf(),
        name: name.clone(),
    })? {
        ServiceResponse::Log(log) => Ok(log),
        other => Err(unexpected(other, project, Some(name))),
    }
}

/// The current status of `project`.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or the project
/// does not load.
pub(crate) fn status(client: &ServiceClient, project: &Path) -> Result<ProjectStatus, CliError> {
    ps(client, Some(project))?
        .into_iter()
        .next()
        .ok_or(CliError::UnexpectedAnswer(EMPTY_STATUS))
}

/// Picks, from a listing of `project`, the process `name` means.
pub fn named<'a>(
    project: &'a Path,
    name: &'a ProcessRef,
) -> impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError> + 'a {
    move |status| resolve(status, project, name)
}

/// Picks, from a listing of `project`, the process `name` means, or the one
/// the person chooses among all of them through `presenter` when the name
/// was left out.
pub fn named_or_chosen<'a>(
    project: &'a Path,
    name: Option<&'a ProcessRef>,
    presenter: Presenter,
) -> impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError> + 'a {
    move |status| match name {
        Some(name) => resolve(status, project, name),
        None => {
            let candidates: Vec<&ProcessStatus> = status.processes().iter().collect();
            presenter.choose_process(&candidates, project)
        },
    }
}

/// The running process of `project` that `name` means, or the one the
/// person chooses among the running ones through `presenter` when the name
/// was left out.
///
/// # Errors
/// Returns [`CliError::NothingRunning`] when there is none to choose, or
/// another [`CliError`] when the service cannot be reached, the name matches
/// no single process, or no process was chosen.
pub fn running_named_or_chosen(
    client: &ServiceClient,
    project: &Path,
    name: Option<&ProcessRef>,
    presenter: Presenter,
) -> Result<ProcessName, CliError> {
    let status = status(client, project)?;
    if let Some(name) = name {
        return resolve(&status, project, name);
    }
    let running: Vec<&ProcessStatus> = status
        .processes()
        .iter()
        .filter(|process| {
            matches!(
                process.state(),
                ProcessState::Running | ProcessState::Paused
            )
        })
        .collect();
    if running.is_empty() {
        return Err(CliError::NothingRunning(project.to_path_buf()));
    }
    presenter.choose_process(&running, project)
}

/// The process of `status` that `name` means.
///
/// # Errors
/// Returns [`CliError::UnknownProcess`] listing every valid name, or
/// [`CliError::AmbiguousProcess`].
pub(crate) fn resolve(
    status: &ProjectStatus,
    project: &Path,
    name: &ProcessRef,
) -> Result<ProcessName, CliError> {
    let names: Vec<ProcessName> = status
        .processes()
        .iter()
        .map(|process| process.name().clone())
        .collect();
    match name.resolve(&names) {
        Ok(found) => Ok(found.clone()),
        Err(UnresolvedProcess::Unknown(typed)) => Err(CliError::UnknownProcess {
            name: typed.to_string(),
            project: project.to_path_buf(),
            valid: names,
        }),
        Err(UnresolvedProcess::Ambiguous(typed, candidates)) => Err(CliError::AmbiguousProcess {
            name: typed.to_string(),
            project: project.to_path_buf(),
            candidates,
        }),
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
/// Returns [`CliError::Interrupted`] when `watch` ended the wait, or another
/// [`CliError`] when the service cannot be reached.
pub(super) fn report(
    client: &ServiceClient,
    project: &Path,
    settling: Settling<'_>,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let (action, before, names, sent, bound) = (
        settling.action(),
        settling.before(),
        settling.names(),
        settling.sent(),
        settling.bound(),
    );
    let stop_grace = names
        .iter()
        .filter_map(|name| find(before, name).and_then(|process| *process.stop_grace()))
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
            .filter(|process| action.in_transition(*process.state()))
            .collect();
        moved.extend(moving.iter().map(|process| process.name().clone()));
        if watch.waiting(action, &moving).is_break() {
            return Err(CliError::Interrupted);
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
        return Err(CliError::Interrupted);
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
                .status(now.map(ProcessView::from))
                .build()
        })
        .collect();
    let (notices, other_notices): (Vec<NoticeView>, Vec<NoticeView>) = notices(client, project)?
        .iter()
        .filter(|notice| *notice.at() >= sent && *notice.level() != NoticeLevel::Attention)
        .map(NoticeView::from)
        .partition(|notice| {
            notice
                .process()
                .as_ref()
                .is_some_and(|process| names.contains(process))
        });
    Ok(ActionReport::builder()
        .project(project.to_path_buf())
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
/// Returns a [`CliError`] when the service cannot be reached.
fn with_refusals(
    report: ActionReport,
    batch: &BatchOutcome,
    client: &ServiceClient,
    project: &Path,
) -> Result<ActionReport, CliError> {
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
/// Returns the [`CliError`] for a refusal or an unexpected answer.
fn batch(response: ServiceResponse, project: &Path) -> Result<BatchOutcome, CliError> {
    match response {
        ServiceResponse::Batch(batch) => Ok(batch),
        other => Err(unexpected(other, project, None)),
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
/// Returns the [`CliError`] for a refusal or an unexpected answer.
pub(crate) fn expect_ok(
    response: ServiceResponse,
    project: &Path,
    name: Option<&ProcessName>,
) -> Result<(), CliError> {
    match response {
        ServiceResponse::Ok => Ok(()),
        other => Err(unexpected(other, project, name)),
    }
}

/// The failure for `response`, which the command did not expect: the
/// refusal it carries, or an answer the command cannot use.
pub(crate) fn unexpected(
    response: ServiceResponse,
    project: &Path,
    name: Option<&ProcessName>,
) -> CliError {
    match response {
        ServiceResponse::Error(body) => refusal(body, project, name),
        other => CliError::UnexpectedAnswer((&other).into()),
    }
}

/// The failure for the service's refusal `body` of a request about
/// `project` and the process `name`, with what to do about it.
fn refusal(body: ServiceErrorBody, project: &Path, name: Option<&ProcessName>) -> CliError {
    match (body.kind(), name) {
        (ServiceErrorKind::UnknownProject, _) => CliError::NoProjectConfig(project.to_path_buf()),
        (ServiceErrorKind::InvalidConfig, _) => CliError::InvalidProjectConfig {
            path: project.to_path_buf(),
            message: body.message().clone(),
        },
        (ServiceErrorKind::UnknownProcess, Some(name)) => CliError::UnknownProcess {
            name: name.to_string(),
            project: project.to_path_buf(),
            valid: body.candidates().clone(),
        },
        (ServiceErrorKind::NotRunning, Some(name)) => CliError::NotRunning(name.clone()),
        (kind, _) => CliError::Refused {
            kind: *kind,
            message: body.message().clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
