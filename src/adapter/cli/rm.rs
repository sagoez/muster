//! `muster rm`: processes removed from a project, those running stopped
//! through their stop policy by the service.

use std::time::SystemTime;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use strum::EnumMessage;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    invocation::stack_editor,
    output::action_title,
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::{
        client::{
            Action, ActionReport, Outcome, ProjectIdentityView, RemovedView, ServiceClient,
            SettleWatch, UNBOUNDED,
            process::{ready_to_send, resolve_among, settle_removed, status},
        },
        store::SqliteStore,
    },
    domain::{
        config::ConfigError,
        port::StackStore,
        project::Project,
        value::{ProcessName, ProcessRef},
    },
};

/// Opens the line naming a removed process.
const REMOVED: &str = "removed";
/// Follows a removed process that was stopped.
const STOPPED: &str = "(stopped)";
/// Between a removed process and what became of it otherwise.
const DETAIL_SEPARATOR: &str = ": ";

/// What `muster rm` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct RmView {
    /// The project the processes were removed from.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// Each removed process, and whether it was stopped.
    #[getset(get = "pub")]
    removed: Vec<RemovedView>,
    /// Whether a process was still running when the wait ended.
    #[serde(skip)]
    #[getset(get_copy = "pub")]
    failed: bool,
    /// What became of each, for the human report.
    #[serde(skip)]
    #[getset(get = "pub")]
    report: ActionReport,
}

/// The processes of `project` that `names` mean, checked all together
/// before anything is removed.
///
/// # Errors
/// Returns the [`CliError`] of the first name that matches no single
/// process; nothing is removed then.
pub fn removable(
    store: &SqliteStore,
    project: &Project,
    names: &[ProcessRef],
) -> Result<Vec<ProcessName>, CliError> {
    let stack = store
        .stack(project.root())?
        .ok_or_else(|| ConfigError::UnknownProject {
            root: project.root().as_ref().clone(),
        })?;
    let mut resolved: Vec<ProcessName> = Vec::new();
    for name in names {
        let found = resolve_among(stack.names(), project, name)?;
        if !resolved.iter().any(|seen| seen.key() == found.key()) {
            resolved.push(found);
        }
    }
    Ok(resolved)
}

/// Removes `names` from `project`, one stored removal each, then waits for
/// the service to stop the ones that ran and reports what became of them.
/// `watch` hears what is still stopping.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached, a removal is
/// refused, or the wait is interrupted.
pub fn remove(
    client: &ServiceClient,
    store: &SqliteStore,
    project: &Project,
    names: &[ProcessName],
    watch: &mut dyn SettleWatch,
) -> Result<RmView, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    let editor = stack_editor(store);
    for name in names {
        editor.remove(project.root(), name)?;
    }
    let report = settle_removed(client, project, &before, names, sent, UNBOUNDED, watch)?;
    Ok(RmView::builder()
        .project(ProjectIdentityView::from(project))
        .removed(report.processes().iter().map(RemovedView::from).collect())
        .failed(report.failed())
        .report(report)
        .build())
}

/// The human report of `view`: one line per removed process, saying it was
/// stopped, or what kept it from stopping.
pub fn rm_rows(view: &RmView) -> Report {
    let mut rows = vec![Row::unlabeled(
        RowKind::Dim,
        view.project().root().display().to_string(),
    )];
    rows.extend(view.report().processes().iter().map(|process| {
        let outcome = process.outcome();
        let line = match outcome {
            Outcome::Stopped => format!("{REMOVED} {} {STOPPED}", process.name()),
            Outcome::WasNotRunning => format!("{REMOVED} {}", process.name()),
            Outcome::Started
            | Outcome::Restarted
            | Outcome::AlreadyRunning
            | Outcome::Exited
            | Outcome::Crashed
            | Outcome::Unsupervised
            | Outcome::StillStarting
            | Outcome::NotStarted
            | Outcome::StillStopping
            | Outcome::StillRunning
            | Outcome::Paused
            | Outcome::AlreadyPaused
            | Outcome::Resumed
            | Outcome::Unchanged
            | Outcome::Refused
            | Outcome::Gone => format!(
                "{REMOVED} {}{DETAIL_SEPARATOR}{}",
                process.name(),
                outcome.get_message().unwrap_or_default()
            ),
        };
        let kind = if outcome.is_failure() {
            RowKind::Fail
        } else {
            RowKind::Ok
        };
        Row::unlabeled(kind, line)
    }));
    Report::new(action_title(Action::Remove, view.project().name()), rows)
        .with_failure(view.failed())
}
