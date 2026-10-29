//! Process commands over the service: `ps`, `up`, `down`, the per-process
//! lifecycle actions, `logs` and `notices`. Each one that acts waits for the
//! processes it touched to settle, then reports what became of them along
//! with the notices supervision raised about them meanwhile.

use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    env,
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
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
    log_read::LogRead,
    notice_view::NoticeView,
    outcome::Outcome,
    presenter::Presenter,
    process_outcome::ProcessOutcome,
    process_view::ProcessView,
    project_identity_view::ProjectIdentityView,
    service_client::ServiceClient,
    settle_watch::SettleWatch,
    settling::{Settling, UNBOUNDED},
    wait_end::WaitEnd,
};
use crate::{
    adapter::{
        holder::rotated_path,
        service::{
            BatchOutcome, Notice, ProcessAction, ProcessStatus, ProjectStatus, ServiceErrorBody,
            ServiceErrorKind, ServiceRequest, ServiceResponse,
        },
    },
    domain::{
        config::ConfigError,
        process::ProcessState,
        project::Project,
        value::{ProcessName, ProcessRef, ProjectRoot, UnresolvedProcess},
    },
};

/// Pause between `ps` polls while waiting for processes to settle.
const SETTLE_POLL: Duration = Duration::from_millis(50);
/// Pause between reads of a followed log.
const FOLLOW_POLL: Duration = Duration::from_millis(200);
/// How an answer to `ps` of one project that listed none is reported.
const EMPTY_STATUS: &str = "a status listing no project";
/// Attempts at reading a log's two generations from one rotation, before
/// reading them as they are.
const LOG_READ_ATTEMPTS: usize = 3;

/// What `ps` reports: the processes of the project at `project`, or of
/// every project the service has loaded when it is `None`.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or the project
/// does not load.
pub fn ps(
    client: &ServiceClient,
    project: Option<&ProjectRoot>,
) -> Result<Vec<ProjectStatus>, CliError> {
    let request = ServiceRequest::Ps {
        project: project.cloned(),
    };
    match client.send(&request)? {
        ServiceResponse::Status(statuses) => Ok(statuses),
        ServiceResponse::Error(body) => Err(refusal(body, None, None)),
        other => Err(CliError::UnexpectedAnswer((&other).into())),
    }
}

/// The environment this command runs in, which the service gives each
/// process it starts for the command.
pub fn caller_environment() -> BTreeMap<OsString, OsString> {
    env::vars_os().collect()
}

/// Starts `project`'s `autostart` processes that are not running and
/// reports each one the service started, once it settled, and each one it
/// left stopped, with why. `watch` hears what is still starting meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn up(
    client: &ServiceClient,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let batch = batch(
        client.send(&ServiceRequest::Up {
            project: project.root().clone(),
            environment: caller_environment(),
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

/// Stops every process of `project` and reports each one the service
/// stopped once it has gone. `watch` hears what is still stopping.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn down(
    client: &ServiceClient,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
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
/// Returns a [`CliError`] when `pick` finds no process, or the service
/// cannot be reached or refuses.
pub fn control(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    action: ProcessAction,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    control_within(client, project, pick, action, UNBOUNDED, watch)
}

/// Like [`control`], but waits at most `bound` for the process to settle and
/// otherwise reports it as still starting or stopping. A stop or kill of a
/// process that is not running is not sent, so a crashed process stays
/// crashed and is reported as not running.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the service
/// cannot be reached or refuses.
pub fn control_within(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    action: ProcessAction,
    bound: Duration,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    let name = pick(&before)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let idle = find(&before, &name).is_some_and(|process| !process.state().may_be_running());
    if !(Action::from(action).halts() && idle) {
        expect_ok(
            client.send(&ServiceRequest::Control {
                project: project.root().clone(),
                name: name.clone(),
                action,
                environment: caller_environment(),
            })?,
            project,
            Some(&name),
        )?;
    }
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

/// Starts `name`, which `muster run` has just added to `project`, and
/// reports it like `start`, telling `watch` while it waits.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn start_added(
    client: &ServiceClient,
    project: &Project,
    name: ProcessName,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    let sent = SystemTime::now();
    expect_ok(
        client.send(&ServiceRequest::Control {
            project: project.root().clone(),
            name: name.clone(),
            action: ProcessAction::Start,
            environment: caller_environment(),
        })?,
        project,
        Some(&name),
    )?;
    let names = [name];
    let settling = Settling::builder()
        .action(Action::Run)
        .before(&before)
        .names(&names)
        .sent(sent)
        .build();
    report(client, project, settling, watch)
}

/// Waits for the processes `names` of `project`, just removed from its
/// stack while listed as `before`, to stop and leave the listing, and
/// reports each as stopped or as not having run.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached.
pub fn settle_removed(
    client: &ServiceClient,
    project: &Project,
    before: &ProjectStatus,
    names: &[ProcessName],
    sent: SystemTime,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let settling = Settling::builder()
        .action(Action::Remove)
        .before(before)
        .names(names)
        .sent(sent)
        .build();
    report(client, project, settling, watch)
}

/// The recent notices of `project` and of the service, oldest first.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or refuses.
pub fn notices(client: &ServiceClient, project: &Project) -> Result<Vec<Notice>, CliError> {
    match client.send(&ServiceRequest::Notices {
        project: project.root().clone(),
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
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    screen: bool,
) -> Result<LogRead, CliError> {
    let name = pick(&status(client, project)?)?;
    let read = LogRead::builder().name(name.clone());
    if screen {
        return match client.send(&ServiceRequest::Screen {
            project: project.root().clone(),
            name: name.clone(),
        })? {
            ServiceResponse::Screen(screen) => {
                Ok(read.log(None).bytes(screen.into_bytes()).build())
            },
            other => Err(unexpected(other, project, Some(&name))),
        };
    }
    let log = log_path(client, project, &name)?;
    let bytes = read_generations(&log)?;
    Ok(read.log(Some(log)).bytes(bytes).build())
}

/// Both generations of `log`, the older first, read from one rotation: the
/// rotated file is opened before the live one, and the read is taken again
/// when a rotation replaced the rotated file in between, so no line is lost
/// or shown twice.
///
/// # Errors
/// Returns [`CliError::Log`] when a file cannot be read.
fn read_generations(log: &Path) -> Result<Vec<u8>, CliError> {
    let rotated_path = rotated_path(log);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let rotated = open_if_present(&rotated_path)?;
        let live = open_if_present(log)?;
        let settled = attempt >= LOG_READ_ATTEMPTS
            || same_file_at(rotated.as_ref(), &rotated_path)
                .map_err(|source| log_error(&rotated_path, source))?;
        if !settled {
            continue;
        }
        let mut bytes = Vec::new();
        for (file, path) in [(rotated, rotated_path.as_path()), (live, log)] {
            if let Some(mut file) = file {
                file.read_to_end(&mut bytes)
                    .map_err(|source| log_error(path, source))?;
            }
        }
        return Ok(bytes);
    }
}

/// `path` opened for reading, or none when it does not exist.
///
/// # Errors
/// Returns [`CliError::Log`] when it exists and cannot be opened.
fn open_if_present(path: &Path) -> Result<Option<File>, CliError> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(log_error(path, source)),
    }
}

/// Whether `path` still names `opened`, or still names nothing when
/// nothing was opened.
///
/// # Errors
/// Returns the [`io::Error`] of a file that cannot be inspected.
fn same_file_at(opened: Option<&File>, path: &Path) -> io::Result<bool> {
    let now = match fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(match (opened, now) {
        (None, None) => true,
        (Some(file), Some(now)) => {
            let then = file.metadata()?;
            then.dev() == now.dev() && then.ino() == now.ino()
        },
        (None, Some(_)) | (Some(_), None) => false,
    })
}

/// The failure of reading the log at `path`.
fn log_error(path: &Path, source: io::Error) -> CliError {
    CliError::Log {
        path: path.to_path_buf(),
        source,
    }
}

/// Copies the log of the process `pick` picks into `out`, the older
/// generation first, then follows it across rotations and restarts until the
/// command is interrupted, waiting for a process that is not running to
/// write. With `json`, each log line is written as one JSON line.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the log cannot be
/// read or written out.
pub fn follow_logs(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    json: bool,
    out: &mut dyn Write,
) -> Result<Infallible, CliError> {
    let name = pick(&status(client, project)?)?;
    let log = log_path(client, project, &name)?;
    if !json {
        return follow(&log, out);
    }
    let template = LogRead::builder()
        .name(name)
        .log(Some(log.clone()))
        .bytes(Vec::new())
        .build()
        .text_in(project.root());
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
        Err(source) => return Err(log_error(&rotated, source)),
    }
    let mut follower = LogFollower::new(log.to_path_buf());
    loop {
        follower.pump(out).map_err(|source| match source.kind() {
            io::ErrorKind::BrokenPipe => CliError::Output(source),
            _ => log_error(log, source),
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
    project: &Project,
    name: &ProcessName,
) -> Result<PathBuf, CliError> {
    match client.send(&ServiceRequest::Log {
        project: project.root().clone(),
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
pub fn status(client: &ServiceClient, project: &Project) -> Result<ProjectStatus, CliError> {
    match client.send(&ServiceRequest::Ps {
        project: Some(project.root().clone()),
    })? {
        ServiceResponse::Status(statuses) => statuses
            .into_iter()
            .next()
            .ok_or(CliError::UnexpectedAnswer(EMPTY_STATUS)),
        other => Err(unexpected(other, project, None)),
    }
}

/// Picks, from a listing of `project`, the process `name` means.
pub fn named<'a>(
    project: &'a Project,
    name: &'a ProcessRef,
) -> impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError> + 'a {
    move |status| resolve(status, project, name)
}

/// Picks, from a listing of `project`, the process `name` means, or the one
/// the person chooses among all of them through `presenter` when the name
/// was left out.
pub fn named_or_chosen<'a>(
    project: &'a Project,
    name: Option<&'a ProcessRef>,
    presenter: Presenter,
) -> impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError> + 'a {
    move |status| match name {
        Some(name) => resolve(status, project, name),
        None => {
            let candidates: Vec<&ProcessStatus> = status.processes().iter().collect();
            presenter.choose_process(&candidates, project.name())
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
    project: &Project,
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
        return Err(CliError::NothingRunning(project.name().clone()));
    }
    presenter.choose_process(&running, project.name())
}

/// The process of `status` that `name` means.
///
/// # Errors
/// Returns [`CliError::UnknownProcess`] listing every valid name, or
/// [`CliError::AmbiguousProcess`].
pub fn resolve(
    status: &ProjectStatus,
    project: &Project,
    name: &ProcessRef,
) -> Result<ProcessName, CliError> {
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
/// Returns [`CliError::UnknownProcess`] listing every valid name, or
/// [`CliError::AmbiguousProcess`].
pub fn resolve_among(
    names: Vec<ProcessName>,
    project: &Project,
    name: &ProcessRef,
) -> Result<ProcessName, CliError> {
    match name.resolve(&names) {
        Ok(found) => Ok(found.clone()),
        Err(UnresolvedProcess::Unknown(typed)) => Err(CliError::UnknownProcess {
            name: typed.to_string(),
            project: project.name().clone(),
            valid: names,
        }),
        Err(UnresolvedProcess::Ambiguous(typed, candidates)) => Err(CliError::AmbiguousProcess {
            name: typed.to_string(),
            project: project.name().clone(),
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
    project: &Project,
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
        .filter(|notice| *notice.at() >= sent)
        .map(NoticeView::from)
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
/// Returns a [`CliError`] when the service cannot be reached.
fn with_refusals(
    report: ActionReport,
    batch: &BatchOutcome,
    client: &ServiceClient,
    project: &Project,
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
fn batch(response: ServiceResponse, project: &Project) -> Result<BatchOutcome, CliError> {
    match response {
        ServiceResponse::Batch(batch) => Ok(batch),
        other => Err(unexpected(other, project, None)),
    }
}

/// The process `name` of `status`.
pub(super) fn find<'a>(status: &'a ProjectStatus, name: &ProcessName) -> Option<&'a ProcessStatus> {
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
pub fn expect_ok(
    response: ServiceResponse,
    project: &Project,
    name: Option<&ProcessName>,
) -> Result<(), CliError> {
    match response {
        ServiceResponse::Ok => Ok(()),
        other => Err(unexpected(other, project, name)),
    }
}

/// The failure for `response`, which the command did not expect: the
/// refusal it carries, or an answer the command cannot use.
pub fn unexpected(
    response: ServiceResponse,
    project: &Project,
    name: Option<&ProcessName>,
) -> CliError {
    match response {
        ServiceResponse::Error(body) => refusal(body, Some(project), name),
        other => CliError::UnexpectedAnswer((&other).into()),
    }
}

/// The failure for the service's refusal `body` of a request about
/// `project` and the process `name`, with what to do about it.
fn refusal(
    body: ServiceErrorBody,
    project: Option<&Project>,
    name: Option<&ProcessName>,
) -> CliError {
    match (body.kind(), project, name) {
        (ServiceErrorKind::UnknownProject, Some(project), _) => {
            CliError::Config(ConfigError::UnknownProject {
                root: project.root().as_ref().clone(),
            })
        },
        (ServiceErrorKind::UnknownProcess, Some(project), Some(name)) => CliError::UnknownProcess {
            name: name.to_string(),
            project: project.name().clone(),
            valid: body.candidates().clone(),
        },
        (ServiceErrorKind::NotRunning, _, Some(name)) => CliError::NotRunning(name.clone()),
        (kind, ..) => CliError::Refused {
            kind: *kind,
            message: body.message().clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

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

    /// Both generations are read older first, byte for byte, including
    /// bytes that are not UTF-8; a missing generation reads as nothing.
    #[test]
    fn both_log_generations_are_read_older_first_and_unchanged() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");

        assert!(read_generations(&log).unwrap().is_empty());
        fs::write(&log, b"live \xff\n").unwrap();
        assert_eq!(read_generations(&log).unwrap(), b"live \xff\n");
        fs::write(rotated_path(&log), b"old\n").unwrap();
        assert_eq!(read_generations(&log).unwrap(), b"old\nlive \xff\n");
    }

    /// A file still named by its path is the same file; one replaced by a
    /// rename, as a rotation does, is not.
    #[test]
    fn a_rotation_between_opens_is_noticed() {
        let scratch = TempDir::new().unwrap();
        let rotated = scratch.path().join("api.log.1");
        let live = scratch.path().join("api.log");
        fs::write(&rotated, b"old").unwrap();
        fs::write(&live, b"new").unwrap();
        let opened = File::open(&rotated).unwrap();

        assert!(same_file_at(Some(&opened), &rotated).unwrap());
        fs::rename(&live, &rotated).unwrap();
        assert!(!same_file_at(Some(&opened), &rotated).unwrap());
        assert!(same_file_at(None, &live).unwrap());
        assert!(!same_file_at(None, &rotated).unwrap());
    }
}
