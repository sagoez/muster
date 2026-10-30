//! The process commands' terminal side: picking a process by asking, and
//! following a log until the command is interrupted.

use std::{
    convert::Infallible,
    io::{self, Write},
    path::Path,
    thread,
    time::Duration,
};

use super::{
    error::CliError, followed_output::FollowedOutput, json_log_lines::JsonLogLines,
    log_follower::LogFollower, presenter::Presenter, rendered_log::RenderedLog,
};
use crate::{
    adapter::{
        client::{
            LogRead, ServiceClient,
            process::{log_error, log_path, log_read_error, resolve, status},
        },
        holder::read_log_stream,
        service::{ProcessStatus, ProjectStatus},
        terminal::colouring::Colouring,
    },
    domain::{
        process::ProcessState,
        project::Project,
        value::{OutputCursor, ProcessName, ProcessRef},
    },
};

/// Pause between reads of a followed log.
const FOLLOW_POLL: Duration = Duration::from_millis(200);

/// Copies the log of the process `pick` picks into `out`, the older
/// generation first, then follows it across rotations and restarts until the
/// command is interrupted, waiting for a process that is not running to
/// write. Each log line is written rendered, coloured as `colouring` asks,
/// or with `json` as one JSON line.
///
/// # Errors
/// Returns a [`CliError`] when `pick` finds no process, or the log cannot be
/// read or written out.
pub fn follow_logs(
    client: &ServiceClient,
    project: &Project,
    pick: impl FnOnce(&ProjectStatus) -> Result<ProcessName, CliError>,
    json: bool,
    colouring: Colouring,
    out: &mut dyn Write,
) -> Result<Infallible, CliError> {
    let name = pick(&status(client, project)?)?;
    let log = log_path(client, project, &name)?;
    let (start, kept) = read_log_stream(&log).map_err(|error| log_read_error(&log, error))?;
    if !json {
        return follow(&log, start, &kept, &mut RenderedLog::new(out, colouring));
    }
    let template = LogRead::builder()
        .name(name)
        .log(Some(log.clone()))
        .end(None)
        .bytes(Vec::new())
        .build()
        .text_in(project);
    follow(
        &log,
        start,
        &kept,
        &mut JsonLogLines::new(out, template, start),
    )
}

/// Writes `kept`, the log's output from `start` on, into `out`, then follows
/// the live log from where it ends until the command is interrupted.
///
/// # Errors
/// Returns a [`CliError`] when the log cannot be read or written out.
fn follow(
    log: &Path,
    start: OutputCursor,
    kept: &[u8],
    out: &mut dyn FollowedOutput,
) -> Result<Infallible, CliError> {
    out.write_all(kept).map_err(CliError::Output)?;
    let mut follower = LogFollower::new(log.to_path_buf(), start.advanced(kept.len() as u64));
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
