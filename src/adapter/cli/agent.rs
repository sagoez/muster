//! Agent session commands over the service: open a new session, close one,
//! and reopen the most recently closed. Each reports once the session has
//! settled, like the process commands.

use std::{path::Path, time::SystemTime};

use super::{
    action::Action,
    action_report::ActionReport,
    error::CliError,
    outcome::Outcome,
    process::{expect_ok, ready_to_send, report, resolve, status, unexpected},
    process_outcome::ProcessOutcome,
    process_view::ProcessView,
    service_client::ServiceClient,
    settle_watch::SettleWatch,
    settling::Settling,
};
use crate::{
    adapter::service::{ProjectStatus, ServiceRequest, ServiceResponse},
    domain::{
        process::AgentTool,
        value::{CommandLine, ProcessName, ProcessRef},
    },
};

/// Opens and starts a new `tool` session in `project`, named `name` or a
/// generated name, running `command` in place of the preset's and resuming
/// through the `resume` template when given. Reports it once it is running or
/// has failed, telling `watch` meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when the service refuses the session or cannot be
/// reached.
pub fn new(
    client: &ServiceClient,
    project: &Path,
    tool: AgentTool,
    name: Option<ProcessName>,
    command: Option<CommandLine>,
    resume: Option<CommandLine>,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    match client.send(&ServiceRequest::AgentNew {
        project: project.to_path_buf(),
        tool,
        name,
        command,
        resume,
    })? {
        ServiceResponse::Agent(opened) => settle(
            client,
            project,
            Action::AgentNew,
            &before,
            opened,
            sent,
            watch,
        ),
        other => Err(unexpected(other, project, None)),
    }
}

/// Closes the agent session `name` of `project` and reports it once its
/// process has gone, telling `watch` meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when the name matches no process, listing the
/// valid ones, or the service refuses or cannot be reached.
pub fn close(
    client: &ServiceClient,
    project: &Path,
    name: &ProcessRef,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    let name = resolve(&before, project, name)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    expect_ok(
        client.send(&ServiceRequest::AgentClose {
            project: project.to_path_buf(),
            name: name.clone(),
        })?,
        project,
        Some(&name),
    )?;
    settle(
        client,
        project,
        Action::AgentClose,
        &before,
        name,
        sent,
        watch,
    )
}

/// Waits for the session `name` to settle after `action` was sent at
/// `sent`, and reports it against `before`, telling `watch` meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or the wait
/// was interrupted.
fn settle(
    client: &ServiceClient,
    project: &Path,
    action: Action,
    before: &ProjectStatus,
    name: ProcessName,
    sent: SystemTime,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let names = [name];
    let settling = Settling::builder()
        .action(action)
        .before(before)
        .names(&names)
        .sent(sent)
        .build();
    report(client, project, settling, watch)
}

/// Reopens `project`'s most recently closed agent session and reports it
/// once running, or as queued when it reopens only after its previous run
/// exits, telling `watch` meanwhile.
///
/// # Errors
/// Returns a [`CliError`] when there is no session to reopen or the service
/// cannot be reached.
pub fn reopen(
    client: &ServiceClient,
    project: &Path,
    watch: &mut dyn SettleWatch,
) -> Result<ActionReport, CliError> {
    let before = status(client, project)?;
    ready_to_send(watch)?;
    let sent = SystemTime::now();
    match client.send(&ServiceRequest::AgentReopen {
        project: project.to_path_buf(),
    })? {
        ServiceResponse::Agent(opened) => settle(
            client,
            project,
            Action::AgentReopen,
            &before,
            opened,
            sent,
            watch,
        ),
        ServiceResponse::AgentQueued(queued) => {
            let now = status(client, project)?;
            let listed = now
                .processes()
                .iter()
                .find(|process| *process.name() == queued)
                .map(ProcessView::from);
            Ok(ActionReport::builder()
                .project(project.to_path_buf())
                .action(Action::AgentReopen)
                .processes(vec![
                    ProcessOutcome::builder()
                        .name(queued)
                        .outcome(Outcome::Queued)
                        .status(listed)
                        .build(),
                ])
                .notices(vec![])
                .other_notices(vec![])
                .build())
        },
        other => Err(unexpected(other, project, None)),
    }
}
