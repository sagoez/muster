//! The open tool: a running process shown to the user in a new pane beside
//! the agent's, in the terminal workspace the agent runs in.

use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use super::{
    process_open_arg::ProcessOpenArg,
    process_tools::{reference, resolved},
    server::{MusterMcp, json, structured, with_alerts},
    tool_error::ToolError,
    tool_result::ToolResult,
};
use crate::{
    adapter::client::{ClientError, OpenRequest, open::open_beside},
    domain::{terminal_workspace::PaneFocus, value::ProcessRef},
};

/// `error` from opening `name`, with what the user can do instead when no
/// pane opened.
fn not_opened(name: &ProcessRef, error: ClientError) -> ToolError {
    match error {
        ClientError::NoWorkspace | ClientError::Workspace(_) => ToolError::NotOpened {
            name: name.clone(),
            source: Box::new(error),
        },
        other => other.into(),
    }
}

#[tool_router(router = open_router, vis = "pub(super)")]
impl MusterMcp {
    /// Shows a running process in a new pane beside the agent's.
    #[tool(
        description = "Show a running process to the user in a new pane beside yours in the \
            terminal workspace you run in, attached to it, without taking the \
            user's focus. read_only=true opens it watch-only. direction is one of \
            <split-directions>; without it the pane splits along its longer side. Returns \
            {project, process, pane, read_only, alerts}. Outside a terminal workspace: isError, \
            kind no-workspace; the message names the muster attach command to ask the user to \
            run in their own terminal. \
            A process that is not running is not-running: start it first with process_start."
    )]
    pub(super) async fn process_open(
        &self,
        Parameters(arg): Parameters<ProcessOpenArg>,
    ) -> ToolResult {
        let name = reference(arg.name())?;
        let request = OpenRequest::builder()
            .binary(self.deps.binary().clone())
            .read_only(arg.read_only().unwrap_or(false))
            .focus(PaneFocus::Keep)
            .direction(arg.split()?)
            .build();
        let Some(workspace) = self.deps.workspace().clone() else {
            return Err(not_opened(&name, ClientError::NoWorkspace).into());
        };
        let (view, alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let resolved = resolved(client, project, &name)?;
                open_beside(client, project, &resolved, workspace.as_ref(), &request)
                    .map_err(|error| not_opened(&name, error))
            })
            .await?;
        structured(with_alerts(json(&view)?, &alerts)?, false)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
        time::Duration,
    };

    use rmcp::model::ErrorCode;
    use serde_json::Value;

    use super::*;
    use crate::{
        adapter::{
            client::{
                fake_service::FakeService, open::attach_command,
                recording_workspace::RecordingWorkspace, served_service::ServedService,
            },
            mcp::test_support::{
                BINARY, Fixture, content, fixture, fixture_with, refusal, register, runtime,
            },
            service::{
                Located, OccupancyStatus, ProcessStatus, ProjectStatus, ServiceErrorBody,
                ServiceErrorKind, ServiceRequest, ServiceResponse,
            },
        },
        domain::{
            config::{McpStackEdits, SettingChange},
            coordination::Author,
            port::{ProjectStore, SettingsStore},
            process::ProcessState,
            project::Project,
            terminal_workspace::{PaneRequest, SplitDirection, WorkspaceError},
            value::{IdleStop, ProcessName, ProjectName, ProjectRoot},
        },
    };

    /// The pane a workspace opens.
    const OPENED: &str = "w1:p4";
    /// `café` with `e` and a combining acute accent, which `café` typed
    /// precomposed only matches by normal form.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    /// A running process `name`, as the service lists it.
    fn listed(name: &str) -> ProcessStatus {
        ProcessStatus::builder()
            .name(ProcessName::try_new(name).unwrap())
            .state(ProcessState::Running)
            .pid(None)
            .uptime(None)
            .restarts(0)
            .usage(None)
            .attached(0)
            .last_exit(None)
            .stop_grace(Duration::from_secs(5))
            .blocked_on(None)
            .build()
    }

    /// A service listing `names` and answering a locate with `located`; it
    /// refuses anything else, as an alerts read may meet.
    fn service(names: &[&str], located: ServiceResponse) -> ServedService {
        let status = ProjectStatus::builder()
            .project(
                Project::builder()
                    .root(ProjectRoot::try_new(PathBuf::from("/srv/shop")).unwrap())
                    .name(ProjectName::try_new("shop").unwrap())
                    .build(),
            )
            .processes(names.iter().copied().map(listed).collect())
            .occupancy(
                OccupancyStatus::builder()
                    .shells(0)
                    .idle_stop(IdleStop::Never)
                    .stops_in(None)
                    .build(),
            )
            .build();
        ServedService::responding(move |request| match request {
            ServiceRequest::Ps { .. } => ServiceResponse::Status(vec![status.clone()]),
            ServiceRequest::Locate { .. } => located.clone(),
            _ => refused(ServiceErrorKind::Internal),
        })
    }

    /// A holder found for the process asked for.
    fn located() -> ServiceResponse {
        ServiceResponse::Located(
            Located::builder()
                .socket(PathBuf::from("/run/holder.sock"))
                .log(PathBuf::from("/state/web.log"))
                .build(),
        )
    }

    /// A refusal of `kind`.
    fn refused(kind: ServiceErrorKind) -> ServiceResponse {
        ServiceResponse::Error(
            ServiceErrorBody::builder()
                .kind(kind)
                .message("refused".to_string())
                .build(),
        )
    }

    /// A registered fixture driving `served` inside `workspace`.
    fn inside(served: &ServedService, workspace: &Arc<RecordingWorkspace>) -> Fixture {
        let fixture = fixture_with(
            FakeService::default().starting(served),
            Some(workspace.clone()),
        );
        register(&fixture);
        fixture
    }

    /// The arguments an agent sends as `json`.
    fn open(json: Value) -> Parameters<ProcessOpenArg> {
        Parameters(serde_json::from_value(json).unwrap())
    }

    /// The pane the fixture's server asks for to show `web`.
    fn pane_for(
        fixture: &Fixture,
        read_only: bool,
        direction: Option<SplitDirection>,
    ) -> PaneRequest {
        let name = ProcessName::try_new("web").unwrap();
        PaneRequest::builder()
            .cwd(fixture.root().as_ref().clone())
            .command(attach_command(
                Path::new(BINARY),
                fixture.root(),
                &name,
                read_only,
            ))
            .focus(PaneFocus::Keep)
            .direction(direction)
            .build()
    }

    /// The name the fixture's project was registered under.
    fn project_name(fixture: &Fixture) -> String {
        fixture
            .store()
            .project_at(fixture.root())
            .unwrap()
            .unwrap()
            .name()
            .to_string()
    }

    /// An agent's open asks for one pane beside its own, attached to the
    /// process and never taking the user's focus, and answers with it.
    #[test]
    fn process_open_opens_beside_the_agent_without_focus() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);

        let answer = runtime().block_on(async {
            content(
                fixture
                    .server()
                    .process_open(open(serde_json::json!({"name": "web"})))
                    .await,
            )
        });

        assert_eq!(workspace.requests(), [pane_for(&fixture, false, None)]);
        assert_eq!(
            answer,
            serde_json::json!({
                "project": project_name(&fixture),
                "process": "web",
                "pane": OPENED,
                "read_only": false,
                "alerts": [],
            })
        );
    }

    /// `read_only` and `direction` reach the pane asked for.
    #[test]
    fn process_open_read_only_reaches_the_request() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);

        let answer = runtime().block_on(async {
            content(
                fixture
                    .server()
                    .process_open(open(
                        serde_json::json!({"name": "web", "read_only": true, "direction": "down"}),
                    ))
                    .await,
            )
        });

        assert_eq!(workspace.requests(), [pane_for(
            &fixture,
            true,
            Some(SplitDirection::Down)
        )]);
        assert_eq!(answer["read_only"], true);
    }

    /// Outside a terminal workspace the open is the agent's to answer, as
    /// `no-workspace` naming the command the user can run, and the service
    /// is never reached or started.
    #[test]
    fn process_open_outside_a_workspace_is_no_workspace_with_the_attach_hint() {
        let fixture = fixture();
        register(&fixture);

        for (name, command) in [
            ("web", "muster attach web"),
            ("-x", "muster attach -- -x"),
            ("$(id) x", "muster attach '$(id) x'"),
        ] {
            let result = runtime().block_on(async {
                fixture
                    .server()
                    .process_open(open(serde_json::json!({"name": name})))
                    .await
            });
            let answer = result.unwrap_err().answer().unwrap();

            assert_eq!(answer.is_error, Some(true));
            let failure = answer.structured_content.unwrap();
            assert_eq!(failure["error"]["kind"], "no-workspace");
            let message = failure["error"]["message"].as_str().unwrap().to_string();
            assert!(
                message.starts_with("not inside a terminal workspace"),
                "{message}"
            );
            assert!(
                message.contains(&format!("run {command} in their own terminal")),
                "{message}"
            );
        }
    }

    /// An unknown, ambiguous or stopped process fails as the service or
    /// the listing says, and no pane is asked for.
    #[test]
    fn process_open_of_a_stopped_process_is_not_running_and_opens_nothing() {
        for (names, located, typed, kind) in [
            (
                vec!["web"],
                refused(ServiceErrorKind::NotRunning),
                "web",
                "not-running",
            ),
            (vec!["web"], located(), "api", "unknown-process"),
            (
                vec![CAFE_DECOMPOSED, CAFE_DECOMPOSED],
                located(),
                "caf\u{e9}",
                "ambiguous",
            ),
        ] {
            let served = service(&names, located);
            let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
            let fixture = inside(&served, &workspace);

            let result = runtime().block_on(async {
                fixture
                    .server()
                    .process_open(open(serde_json::json!({"name": typed})))
                    .await
            });
            let answer = result.unwrap_err().answer().unwrap();

            assert_eq!(answer.is_error, Some(true), "{typed}");
            assert_eq!(answer.structured_content.unwrap()["error"]["kind"], kind);
            assert!(workspace.requests().is_empty(), "{typed}");
        }
    }

    /// The pane it opens is a view, not a stack edit, so a read-only
    /// `mcp-stack-edits` lets it through.
    #[test]
    fn process_open_is_allowed_when_stack_edits_are_read_only() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);
        fixture
            .store()
            .apply(
                &SettingChange::McpStackEdits(McpStackEdits::ReadOnly),
                &Author::human(),
            )
            .unwrap();

        let answer = runtime().block_on(async {
            content(
                fixture
                    .server()
                    .process_open(open(serde_json::json!({"name": "web"})))
                    .await,
            )
        });

        assert_eq!(answer["pane"], OPENED);
        assert_eq!(workspace.requests().len(), 1);
    }

    /// A direction naming no side is the agent's mistake naming
    /// `direction` and listing the sides, refused before any pane.
    #[test]
    fn a_bad_direction_is_invalid_input_naming_direction() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);

        let text = runtime().block_on(async {
            refusal(
                fixture
                    .server()
                    .process_open(open(
                        serde_json::json!({"name": "web", "direction": "left"}),
                    ))
                    .await,
            )
        });

        assert_eq!(text, "invalid direction: expected right, down, got 'left'");
        assert!(workspace.requests().is_empty());
    }

    /// A workspace that does not open the pane is muster's failure,
    /// `workspace-failed`, its text made safe to show and naming the
    /// command the user can run.
    #[test]
    fn a_workspace_failure_is_workspace_failed_and_sanitized() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::failing(WorkspaceError::Refused {
            workspace: "tiles".to_string(),
            action: "split a pane".to_string(),
            code: "pane_not_found".to_string(),
            message: "no\u{1b}]0;owned\u{7} pane".to_string(),
        }));
        let fixture = inside(&served, &workspace);

        let error = runtime()
            .block_on(async {
                fixture
                    .server()
                    .process_open(open(serde_json::json!({"name": "web"})))
                    .await
            })
            .unwrap_err()
            .answer()
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        let failure = error.data.expect("the failure rides along");
        assert_eq!(failure["error"]["kind"], "workspace-failed");
        let message = failure["error"]["message"].as_str().unwrap();
        assert!(
            !message.contains('\u{1b}') && !message.contains('\u{7}'),
            "{message:?}"
        );
        assert!(
            message.contains("no\\u{1b}]0;owned\\u{7} pane"),
            "{message}"
        );
        assert!(
            message.contains("run muster attach web in their own terminal"),
            "{message}"
        );
        assert_eq!(workspace.requests().len(), 1);
    }

    /// The tool's description and arguments, as an agent reads them.
    #[test]
    fn the_open_tool_reads_as_described() {
        let fixture = fixture();
        let tool = fixture
            .server()
            .tool_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "process_open")
            .unwrap();

        insta::assert_snapshot!(
            "process_open_description",
            tool.description.clone().unwrap_or_default()
        );
        insta::assert_snapshot!(
            "process_open_schema",
            serde_json::to_string_pretty(&Value::Object(tool.input_schema.as_ref().clone()))
                .unwrap()
        );
    }
}
