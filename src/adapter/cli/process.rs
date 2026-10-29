//! The process commands' terminal side: picking a process by asking, and
//! following a log until the command is interrupted.

use std::{
    convert::Infallible,
    fs,
    io::{self, Write},
    path::Path,
    thread,
    time::Duration,
};

use super::{
    error::CliError, json_log_lines::JsonLogLines, log_follower::LogFollower, presenter::Presenter,
};
use crate::{
    adapter::{
        client::{
            LogRead, ServiceClient,
            process::{log_error, log_path, resolve, status},
        },
        holder::rotated_path,
        service::{ProcessStatus, ProjectStatus},
    },
    domain::{
        process::ProcessState,
        project::Project,
        value::{ProcessName, ProcessRef},
    },
};

/// Pause between reads of a followed log.
const FOLLOW_POLL: Duration = Duration::from_millis(200);

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
        Err(source) => return Err(log_error(&rotated, source).into()),
    }
    let mut follower = LogFollower::new(log.to_path_buf());
    loop {
        follower.pump(out).map_err(|source| match source.kind() {
            io::ErrorKind::BrokenPipe => CliError::Output(source),
            _ => log_error(log, source).into(),
        })?;
        out.flush().map_err(CliError::Output)?;
        thread::sleep(FOLLOW_POLL);
    }
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
        Some(name) => Ok(resolve(status, project, name)?),
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
        return Ok(resolve(&status, project, name)?);
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
