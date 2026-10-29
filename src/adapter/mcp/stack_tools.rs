//! The stack tools: the whole picture, bringing it up, and its events.

use std::time::Duration;

use rmcp::{
    RoleServer, handler::server::wrapper::Parameters, service::RequestContext, tool, tool_router,
};

use super::{
    agent_wait_view::AgentWaitView,
    bounds::{CALL_BOUND, DEFAULT_WAIT, TIMEOUT_FIELD, WAIT_S_FIELD, wait_bound},
    events_view::EventsView,
    occupancy_view::OccupancyView,
    process_fields::choice,
    progress::with_progress,
    project_arg::ProjectArg,
    server::{MusterMcp, json, structured, with_alerts},
    stack_events_arg::StackEventsArg,
    stack_process_view::StackProcessView,
    stack_status_view::StackStatusView,
    stack_up_arg::StackUpArg,
    tool_error::ToolError,
    tool_result::ToolResult,
    up_view::UpView,
};
use crate::{
    adapter::{
        client::{LockView, NoticeView, ProjectIdentityView, Unwatched, process},
        clock::epoch_now,
        service::{EventKind, EventSeq, Notice, ProjectStatus, WaitSpec},
    },
    domain::{coordination::Lock, value::EpochSeconds},
};

/// Most notices `stack_status` returns.
const STATUS_NOTICES: usize = 10;
/// The `since` field of `stack_events`.
const SINCE_FIELD: &str = "since";
/// The `kinds` field of `stack_events`.
const KINDS_FIELD: &str = "kinds";

#[tool_router(router = stack_router, vis = "pub(super)")]
impl MusterMcp {
    /// The whole stack of one project.
    #[tool(
        description = "The whole stack of this project in one call: each process's state, \
            readiness, uptime, restarts, listening ports, URLs, CPU and memory, and how it last \
            exited, plus recent warnings, held locks and alerts. Call this first, and whenever \
            you are unsure what is running. Returns {project, you, occupancy: {shells, \
            idle_stop, stops_in_s}, processes: [{name, state, ready, pid, uptime_s, restarts, \
            ports, urls, run_start, cpu_percent, memory_bytes, last_exit: {status, signal, \
            reason, at}, blocked_on: {port, holder, hint}}], notices, locks, events_cursor, \
            alerts}; a field with no value is left out. occupancy counts the user's shells in \
            the project; with none, the project is stopped idle_stop seconds after the last \
            one left (null idle_stop: never), stops_in_s from now. blocked_on names the first declared or readiness port that \
            was held when its start was refused. Pass run_start as since to process_logs to \
            read only the current run."
    )]
    pub(super) async fn stack_status(&self, Parameters(arg): Parameters<ProjectArg>) -> ToolResult {
        let coordination = self.deps.coordination().clone();
        let edits = self.deps.edits().clone();
        let ((status, notices, locks), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let status = process::status(client, project)?;
                let notices = process::notices(client, edits.as_ref(), project)?;
                let locks = coordination.locks(project.root())?;
                Ok((status, notices, locks))
            })
            .await?;
        let view = self.status_view(&status, &notices, &locks, epoch_now());
        structured(with_alerts(json(&view)?, &alerts)?, false)
    }

    /// Starts every autostart process of one project.
    #[tool(
        description = "Start every autostart process of this project that is not running. With \
            wait=true, waits until each one is ready (or running, when it has no readiness \
            check), at most timeout_s (default <default-wait-s>, at most <call-bound-s>). \
            Returns {project, started, already_running, refused: [{name, outcome, reason, \
            block, status}], others (crashed, exited or not settled in time, each shaped as \
            refused), notices, waits, alerts}. block is {port, holder} when a held port kept \
            it from starting, holder being {kind: supervised, project, process} or {kind: \
            other, pid, command}, and null otherwise; status is how muster ps --json lists it \
            now, null once it is no longer listed. A \
            port found held after the answer shows as blocked_on in stack_status. Flagged \
            isError when any process failed to start or crashed while waiting."
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
        description = "Lifecycle events of this project after the cursor since: <event-kinds>. \
            idle-stopped means the project was stopped because no shell was in it for its idle \
            stop. Without since, returns the last <events-without-since>. With wait_s (at most \
            <call-bound-s>), blocks until at least one new event arrives. Returns {events: \
            [{seq, at, process, kind, detail}], cursor, gap}; pass cursor as since next time. gap=true means events \
            after since were missed, dropped or recorded by an earlier service, and comes back \
            at once with every event held; re-read stack_status then. Reading events never \
            marks alerts heard."
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
    /// What `stack_status` answers for `status`, its project's `notices` and
    /// held `locks`, the locks' expiry read at `now`.
    fn status_view(
        &self,
        status: &ProjectStatus,
        notices: &[Notice],
        locks: &[Lock],
        now: EpochSeconds,
    ) -> StackStatusView {
        let recent = notices.len().saturating_sub(STATUS_NOTICES);
        StackStatusView::builder()
            .project(ProjectIdentityView::from(status.project()))
            .you(self.identity.author().label().to_string())
            .occupancy(OccupancyView::from(status.occupancy()))
            .processes(
                status
                    .processes()
                    .iter()
                    .map(StackProcessView::from)
                    .collect(),
            )
            .notices(
                notices[recent..]
                    .iter()
                    .map(|notice| NoticeView::for_project(notice, status.project()))
                    .collect(),
            )
            .locks(locks.iter().map(|lock| LockView::at(lock, now)).collect())
            .events_cursor(
                self.alerts
                    .since(status.project().root())
                    .map(|cursor| cursor.to_string()),
            )
            .build()
    }

    /// What `stack_up` does, without its progress notifications.
    ///
    /// # Errors
    /// Returns the caller's mistake or a fault of muster's.
    pub(super) async fn up(&self, arg: StackUpArg) -> ToolResult {
        let bound = wait_bound(TIMEOUT_FIELD, arg.timeout_s(), DEFAULT_WAIT)?;
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
            .map(|raw| choice::<EventKind>(KINDS_FIELD, raw))
            .collect::<Result<Vec<_>, _>>()?;
        let wait = wait_bound(WAIT_S_FIELD, arg.wait_s(), Duration::ZERO)?;
        let project = arg.project().clone();
        let page = self
            .blocking(move |deps| {
                let project = deps.project(project.as_deref())?;
                let client = deps.service().connect()?;
                Ok(process::events(&client, &project, since, &kinds, wait)?)
            })
            .await?;
        structured(json(&EventsView::from(&page))?, false)
    }
}

#[cfg(test)]
mod tests {
    use std::iter;

    use rmcp::model::ErrorCode;
    use serde_json::Value;
    use strum::IntoEnumIterator;

    use super::*;
    use crate::{
        adapter::{
            client::{Outcome, PortBlockView, ProcessOutcome},
            mcp::test_support::{Fixture, answered, fixture, refusal, register, runtime},
            service::{OccupancyStatus, PortBlock, PortHolder, PortOwner, ProcessStatus},
        },
        domain::{
            port::{PortOccupant, ProjectStore},
            process::ProcessState,
            value::{IdleStop, LocalPort, Pid, PositiveDuration, ProcessName, ProjectName},
        },
    };

    /// The fixture's registered project with `processes` in it and
    /// `occupancy`.
    fn status_of(
        fixture: &Fixture,
        processes: Vec<ProcessStatus>,
        occupancy: OccupancyStatus,
    ) -> ProjectStatus {
        ProjectStatus::builder()
            .project(
                fixture
                    .store()
                    .project_at(fixture.root())
                    .unwrap()
                    .expect("a registered project"),
            )
            .processes(processes)
            .occupancy(occupancy)
            .build()
    }

    /// `shells` in a project with a 30 minute idle stop, its stop `stops_in`
    /// away.
    fn occupancy(shells: u32, stops_in: Option<Duration>) -> OccupancyStatus {
        OccupancyStatus::builder()
            .shells(shells)
            .idle_stop(IdleStop::After(
                PositiveDuration::try_new(Duration::from_secs(1_800)).unwrap(),
            ))
            .stops_in(stops_in)
            .build()
    }

    /// `name`, not running, its last start refused by `block`.
    fn blocked(name: &str, block: PortBlock) -> ProcessStatus {
        ProcessStatus::builder()
            .name(ProcessName::try_new(name).unwrap())
            .state(ProcessState::Blocked)
            .pid(None)
            .uptime(None)
            .restarts(0)
            .usage(None)
            .attached(0)
            .last_exit(None)
            .stop_grace(Duration::from_secs(5))
            .blocked_on(Some(block))
            .build()
    }

    /// What `stack_status` answers for `status`, as the agent reads it, with
    /// the scratch root replaced so a snapshot holds still.
    fn status_text(fixture: &Fixture, status: &ProjectStatus) -> String {
        let view = fixture
            .server()
            .status_view(status, &[], &[], EpochSeconds::new(0));
        serde_json::to_string_pretty(&view)
            .unwrap()
            .replace(&fixture.root().as_ref().display().to_string(), "<root>")
    }

    /// [`status_text`] read back as JSON.
    fn answered_status(fixture: &Fixture, status: &ProjectStatus) -> Value {
        serde_json::from_str(&status_text(fixture, status)).unwrap()
    }

    /// `stack_status` reports how many shells are in the project and its
    /// idle stop; the seconds until a pending stop, rounded up, appear only
    /// while one is pending.
    #[test]
    fn stack_status_reports_occupancy_and_omits_stops_in_without_a_timer() {
        let fixture = fixture();
        register(&fixture);

        let occupied = status_text(&fixture, &status_of(&fixture, vec![], occupancy(2, None)));
        let pending = status_text(
            &fixture,
            &status_of(
                &fixture,
                vec![],
                occupancy(0, Some(Duration::from_millis(1_733_200))),
            ),
        );

        assert!(!occupied.contains("stops_in_s"), "{occupied}");
        insta::assert_snapshot!("stack_status_occupied", occupied);
        insta::assert_snapshot!("stack_status_idle", pending);
    }

    /// A process declaring two ports, one held by another project's process
    /// and one by an outside program, shows the first held port the service
    /// refused it for, its holder and the call that frees it.
    #[test]
    fn a_blocked_process_with_two_ports_names_the_first_held() {
        let fixture = fixture();
        register(&fixture);
        let by_admin = PortBlock::builder()
            .port(LocalPort::try_new(3000).unwrap())
            .holder(PortOwner::Supervised(
                PortHolder::builder()
                    .project(ProjectName::try_new("admin").unwrap())
                    .process(ProcessName::try_new("web").unwrap())
                    .build(),
            ))
            .build();
        let by_outsider = PortBlock::builder()
            .port(LocalPort::try_new(5173).unwrap())
            .holder(PortOwner::Other(
                PortOccupant::builder()
                    .pid(Pid::try_new(4242).unwrap())
                    .command("python3 -m http.server 5173".to_string())
                    .build(),
            ))
            .build();

        let answer = answered_status(
            &fixture,
            &status_of(
                &fixture,
                vec![blocked("web", by_admin), blocked("docs", by_outsider)],
                occupancy(1, None),
            ),
        );

        assert_eq!(
            answer["processes"][0]["blocked_on"],
            serde_json::json!({
                "port": 3000,
                "holder": "project 'admin', process 'web'",
                "hint": "stop it with process_stop {project: 'admin', name: 'web'}",
            })
        );
        assert_eq!(
            answer["processes"][1]["blocked_on"],
            serde_json::json!({
                "port": 5173,
                "holder": "pid 4242 (python3 -m http.server 5173)",
            })
        );
        assert_eq!(answer["processes"][0]["state"], "blocked");
    }

    /// `stack_events` takes `idle-stopped` as a kind, going on to ask the
    /// service, and its description lists it with every other kind.
    #[test]
    fn stack_events_accepts_idle_stopped() {
        let fixture = fixture();
        register(&fixture);
        runtime().block_on(async {
            let unreachable = answered(
                fixture
                    .server()
                    .events(
                        StackEventsArg::builder()
                            .kinds(Some(vec!["idle-stopped".to_string()]))
                            .build(),
                    )
                    .await,
            )
            .unwrap_err();
            assert_eq!(unreachable.code, ErrorCode::INTERNAL_ERROR);
        });
        let tool = fixture
            .server()
            .tool_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "stack_events")
            .unwrap();
        let description = tool.description.unwrap_or_default();
        for kind in EventKind::iter() {
            assert!(
                description.contains(&kind.to_string()),
                "{kind}: {description}"
            );
        }
        let schema = Value::Object(tool.input_schema.as_ref().clone());
        let kinds = schema["properties"]["kinds"]["description"].to_string();
        assert!(kinds.contains("idle-stopped"), "{kinds}");
    }

    /// The keys of `value`, and of every object nested in it.
    fn keys(value: &Value) -> Vec<String> {
        match value {
            Value::Object(fields) => fields
                .iter()
                .flat_map(|(key, value)| iter::once(key.clone()).chain(keys(value)))
                .collect(),
            Value::Array(items) => items.iter().flat_map(keys).collect(),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
        }
    }

    /// `stack_up`'s description names every field a refused entry carries,
    /// its block and each kind of holder included.
    #[test]
    fn stack_up_describes_the_refused_shape() {
        let fixture = fixture();
        let description = fixture
            .server()
            .tool_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "stack_up")
            .unwrap()
            .description
            .unwrap_or_default();
        let port = LocalPort::try_new(5173).unwrap();
        let holders = [
            PortOwner::Supervised(
                PortHolder::builder()
                    .project(ProjectName::try_new("admin").unwrap())
                    .process(ProcessName::try_new("web").unwrap())
                    .build(),
            ),
            PortOwner::Other(
                PortOccupant::builder()
                    .pid(Pid::try_new(4242).unwrap())
                    .command("python3".to_string())
                    .build(),
            ),
        ];
        for holder in holders {
            let refused = ProcessOutcome::builder()
                .name(ProcessName::try_new("web").unwrap())
                .outcome(Outcome::Refused)
                .reason(Some("port 5173 is held".to_string()))
                .block(Some(PortBlockView::from(
                    &PortBlock::builder().port(port).holder(holder).build(),
                )))
                .status(None)
                .build();
            let json = serde_json::to_value(&refused).unwrap();
            for key in keys(&json) {
                assert!(description.contains(&key), "{key}: {description}");
            }
            let kind = json["block"]["holder"]["kind"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(
                description.contains(&format!("kind: {kind}")),
                "{kind}: {description}"
            );
        }
    }

    /// A malformed cursor or kind is the caller's mistake, refused before
    /// any service is reached.
    #[test]
    fn a_malformed_cursor_or_kind_is_refused() {
        let fixture = fixture();
        runtime().block_on(async {
            let cursor = refusal(
                fixture
                    .server()
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
                    .server()
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
                    .server()
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
                    .server()
                    .stack_status(Parameters(ProjectArg::default()))
                    .await,
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        });
    }
}
