//! The process use cases both drivers run over the service: listing, `up`,
//! `down`, the per-process lifecycle actions, removal, logs and notices.
//! Each one that acts waits for the processes it touched to settle, then
//! reports what became of them along with the notices supervision raised
//! about them meanwhile.

use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    action::Action,
    action_report::ActionReport,
    client_error::ClientError,
    log_read::LogRead,
    notice_view::NoticeView,
    outcome::Outcome,
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
            BatchOutcome, LaunchEnvironment, Notice, ProcessAction, ProcessStatus, ProjectStatus,
            ServiceErrorBody, ServiceErrorKind, ServiceRequest, ServiceResponse,
        },
    },
    domain::{
        config::ConfigError,
        project::Project,
        value::{ProcessName, ProcessRef, ProjectRoot, UnresolvedProcess},
    },
};

/// Pause between `ps` polls while waiting for processes to settle.
const SETTLE_POLL: Duration = Duration::from_millis(50);
/// How an answer to `ps` of one project that listed none is reported.
const EMPTY_STATUS: &str = "a status listing no project";
/// Attempts at reading a log's two generations from one rotation, before
/// reading them as they are.
const LOG_READ_ATTEMPTS: usize = 3;

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
        other => Err(ClientError::UnexpectedAnswer((&other).into())),
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
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let batch = batch(
        client.send(&ServiceRequest::Up {
            project: project.root().clone(),
            environment: LaunchEnvironment::current(),
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
        })?,
        project,
        Some(name),
    )
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

/// Whether a service that already runs has `project` loaded and lists
/// `name` as possibly running. It never starts the service, and an answer it
/// cannot get reads as not running, since this only tells a caller whether
/// an edit it already made waits for a restart.
pub fn runs_now(project: &ProjectRoot, name: &ProcessName) -> bool {
    ServiceClient::loaded_status(project)
        .ok()
        .flatten()
        .is_some_and(|status| {
            status.processes().iter().any(|process| {
                process.name().key() == name.key() && process.state().may_be_running()
            })
        })
}

/// The recent notices of `project` and of the service, oldest first.
///
/// # Errors
/// Returns a [`ClientError`] when the service cannot be reached or refuses.
pub fn notices(client: &ServiceClient, project: &Project) -> Result<Vec<Notice>, ClientError> {
    match client.send(&ServiceRequest::Notices {
        project: project.root().clone(),
    })? {
        ServiceResponse::Notices(notices) => Ok(notices),
        other => Err(unexpected(other, project, None)),
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
                Ok(read.log(None).bytes(screen.into_bytes()).build())
            },
            other => Err(unexpected(other, project, Some(&name)).into()),
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
/// Returns [`ClientError::Log`] when a file cannot be read.
fn read_generations(log: &Path) -> Result<Vec<u8>, ClientError> {
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
/// Returns [`ClientError::Log`] when it exists and cannot be opened.
fn open_if_present(path: &Path) -> Result<Option<File>, ClientError> {
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
pub fn log_error(path: &Path, source: io::Error) -> ClientError {
    ClientError::Log {
        path: path.to_path_buf(),
        source,
    }
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
        other => Err(unexpected(other, project, Some(name))),
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
        other => Err(unexpected(other, project, None)),
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
            .filter(|process| action.in_transition(*process.state()))
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
/// Returns the [`ClientError`] for a refusal or an unexpected answer.
pub fn expect_ok(
    response: ServiceResponse,
    project: &Project,
    name: Option<&ProcessName>,
) -> Result<(), ClientError> {
    match response {
        ServiceResponse::Ok => Ok(()),
        other => Err(unexpected(other, project, name)),
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
        other => ClientError::UnexpectedAnswer((&other).into()),
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
