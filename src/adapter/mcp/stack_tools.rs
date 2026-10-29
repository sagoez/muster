//! The stack tools: the whole picture, bringing it up, and its events.

use std::time::Duration;

use rmcp::{
    RoleServer, handler::server::wrapper::Parameters, service::RequestContext, tool, tool_router,
};

use super::{
    agent_wait_view::AgentWaitView,
    bounds::{CALL_BOUND, wait_bound},
    events_view::EventsView,
    lock_view::LockView,
    process_fields::invalid,
    progress::with_progress,
    project_arg::ProjectArg,
    server::{MusterMcp, ToolResult, json, structured, unix_now, with_alerts},
    stack_events_arg::StackEventsArg,
    stack_process_view::StackProcessView,
    stack_status_view::StackStatusView,
    stack_up_arg::StackUpArg,
    tool_error::ToolError,
    up_view::UpView,
};
use crate::adapter::{
    client::{NoticeView, ProjectIdentityView, Unwatched, process},
    service::{EventKind, EventSeq, WaitSpec},
};

/// Most notices `stack_status` returns.
const STATUS_NOTICES: usize = 10;
/// The `since` field of `stack_events`.
const SINCE_FIELD: &str = "since";
/// The `kinds` field of `stack_events`.
const KINDS_FIELD: &str = "kinds";
/// The kinds `stack_events` takes.
const KINDS_FORM: &str =
    "started, ready, ready-timeout, exited, crashed, restarting, stopped, blocked or notice";

#[tool_router(router = stack_router, vis = "pub(super)")]
impl MusterMcp {
    /// The whole stack of one project.
    #[tool(
        description = "The whole stack of this project in one call: each process's state, \
            uptime, restarts, CPU and memory, and how it last exited, plus recent warnings, held \
            locks and alerts. Call this first, and whenever you are unsure what is running. \
            Returns {project, you, processes: [{name, state, pid, uptime_s, restarts, \
            run_start, cpu_percent, memory_bytes, last_exit}], notices, locks, events_cursor, \
            alerts}; a field with no value is left out."
    )]
    pub(super) async fn stack_status(&self, Parameters(arg): Parameters<ProjectArg>) -> ToolResult {
        let coordination = self.deps.coordination().clone();
        let ((status, notices, locks), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let status = process::status(client, project)?;
                let notices = process::notices(client, project)?;
                let locks = coordination.locks(project.root())?;
                Ok((status, notices, locks))
            })
            .await?;
        let now = unix_now();
        let recent = notices.len().saturating_sub(STATUS_NOTICES);
        let view = StackStatusView::builder()
            .project(ProjectIdentityView::from(status.project()))
            .you(self.identity.author().label().to_string())
            .processes(
                status
                    .processes()
                    .iter()
                    .map(StackProcessView::from)
                    .collect(),
            )
            .notices(notices[recent..].iter().map(NoticeView::from).collect())
            .locks(locks.iter().map(|lock| LockView::at(lock, now)).collect())
            .events_cursor(
                self.alerts
                    .since(status.project().root())
                    .map(|cursor| cursor.to_string()),
            )
            .build();
        structured(with_alerts(json(&view)?, &alerts)?, false)
    }

    /// Starts every autostart process of one project.
    #[tool(
        description = "Start every autostart process of this project that is not running. With \
            wait=true, waits until each one is ready (or running, when it has no readiness \
            check), at most timeout_s (default 30, at most 45). Returns {project, started, \
            already_running, refused: [{name, reason}], others: [{name, outcome}] (crashed, \
            exited or not settled in time), notices, waits, alerts}. Flagged isError when any \
            process failed to start or crashed while waiting."
    )]
    pub(super) async fn stack_up(
        &self,
        Parameters(arg): Parameters<StackUpArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.up(arg)).await
    }

    /// Lifecycle events of one project.
    #[tool(
        description = "Lifecycle events of this project after the cursor since: started, ready, \
            ready-timeout, exited, crashed, restarting, stopped, blocked, notice. Without since, \
            returns the last 20. With wait_s (at most 45), blocks until at least one new event \
            arrives. Returns {events: [{seq, at, process, kind, detail}], cursor, gap, alerts}; \
            pass cursor as since next time. gap=true means older events were dropped."
    )]
    pub(super) async fn stack_events(
        &self,
        Parameters(arg): Parameters<StackEventsArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.events(arg)).await
    }
}

impl MusterMcp {
    /// What `stack_up` does, without its progress notifications.
    ///
    /// # Errors
    /// Returns the caller's mistake or a fault of muster's.
    pub(super) async fn up(&self, arg: StackUpArg) -> ToolResult {
        let bound = wait_bound(arg.timeout_s());
        let wait = arg
            .wait()
            .unwrap_or(false)
            .then(|| WaitSpec::builder().timeout(bound).build());
        let ((report, waits), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                Ok(match &wait {
                    Some(spec) => {
                        process::up_waiting(client, project, CALL_BOUND, spec, &mut Unwatched)?
                    },
                    None => (
                        process::up_within(client, project, CALL_BOUND, &mut Unwatched)?,
                        Vec::new(),
                    ),
                })
            })
            .await?;
        let failed = report.failed() || waits.iter().any(|wait| wait.status().is_failure());
        let view =
            UpView::from(&report).with_waits(waits.iter().map(AgentWaitView::from).collect());
        structured(with_alerts(json(&view)?, &alerts)?, failed)
    }

    /// What `stack_events` does, without its progress notifications.
    ///
    /// # Errors
    /// Returns the caller's mistake for a malformed cursor or kind, or a
    /// fault of muster's.
    pub(super) async fn events(&self, arg: StackEventsArg) -> ToolResult {
        let since = arg
            .since()
            .as_deref()
            .map(|raw| raw.parse::<EventSeq>())
            .transpose()
            .map_err(|reason| ToolError::InvalidField {
                field: SINCE_FIELD,
                reason,
            })?;
        let kinds = arg
            .kinds()
            .iter()
            .flatten()
            .map(|raw| {
                raw.trim()
                    .parse::<EventKind>()
                    .map_err(|_| invalid(KINDS_FIELD, KINDS_FORM, raw))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let wait = arg
            .wait_s()
            .map_or(Duration::ZERO, Duration::from_secs)
            .min(CALL_BOUND);
        let alerts_seen = self.alerts.clone();
        let (page, alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let page = process::events(client, project, since, &kinds, wait)?;
                alerts_seen.mark_seen(project.root(), *page.cursor());
                Ok(page)
            })
            .await?;
        structured(
            with_alerts(json(&EventsView::from(&page))?, &alerts)?,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::mcp::test_support::{answered, fixture, refusal, register, runtime};

    /// A malformed cursor or kind is the caller's mistake, refused before
    /// any service is reached.
    #[test]
    fn a_malformed_cursor_or_kind_is_refused() {
        let fixture = fixture();
        runtime().block_on(async {
            let cursor = refusal(
                fixture
                    .server
                    .events(
                        StackEventsArg::builder()
                            .since(Some("9921".to_string()))
                            .build(),
                    )
                    .await,
            );
            assert!(
                cursor.contains("since") && cursor.contains("e:"),
                "{cursor}"
            );
            let kind = refusal(
                fixture
                    .server
                    .events(
                        StackEventsArg::builder()
                            .kinds(Some(vec!["exploded".to_string()]))
                            .build(),
                    )
                    .await,
            );
            assert!(kind.contains("crashed"), "{kind}");
        });
    }

    /// Outside every project, the stack tools say so and name process_add.
    #[test]
    fn stack_status_outside_a_project_names_process_add() {
        let fixture = fixture();
        runtime().block_on(async {
            let text = refusal(
                fixture
                    .server
                    .stack_status(Parameters(ProjectArg::default()))
                    .await,
            );
            assert!(text.contains("no muster project contains"), "{text}");
            assert!(text.contains("process_add"), "{text}");
        });
    }

    /// A service that cannot be reached is muster's fault, a protocol error,
    /// and never the user's real service.
    #[test]
    fn an_unreachable_service_is_a_protocol_error() {
        let fixture = fixture();
        register(&fixture);
        runtime().block_on(async {
            let error = answered(
                fixture
                    .server
                    .stack_status(Parameters(ProjectArg::default()))
                    .await,
            )
            .unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        });
    }
}
