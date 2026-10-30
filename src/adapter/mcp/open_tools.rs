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
    domain::{project::Project, terminal_workspace::PaneFocus, value::ProcessName},
};

/// `error` from opening the running process `name` of `project` as
/// `request` asks, with what the user can do instead when no pane opened.
fn not_opened(
    project: &Project,
    name: &ProcessName,
    request: &OpenRequest,
    error: ClientError,
) -> ToolError {
    match error {
        ClientError::NoWorkspace | ClientError::Workspace(_) | ClientError::BinaryGone(_) => {
            ToolError::NotOpened {
                root: project.root().clone(),
                name: name.clone(),
                read_only: request.read_only(),
                source: Box::new(error),
            }
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
            <split-directions>; without it, right when the pane is wider than tall and each \
            half keeps at least <split-min-half-cols> columns, and down otherwise. Returns \
            {project, process, pane, read_only, alerts}. The process is resolved first: an \
            unknown name is unknown-process with candidates, and a process that is not running \
            is not-running: start it first with process_start. Outside a terminal workspace, a \
            running process is isError, kind no-workspace; the message names the muster attach \
            command to ask the user to run in their own terminal. A workspace that does not open \
            the pane is an error of kind workspace-failed naming the same command."
    )]
    pub(super) async fn process_open(
        &self,
        Parameters(arg): Parameters<ProcessOpenArg>,
    ) -> ToolResult {
        let name = reference(arg.name())?;
        let request = OpenRequest::builder()
            .binary((self.deps.binary())())
            .read_only(arg.read_only().unwrap_or(false))
            .focus(PaneFocus::Keep)
            .direction(arg.split()?)
            .build();
        let workspace = self.deps.terminal_workspace().clone();
        let (view, alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let resolved = resolved(client, project, &name)?;
                open_beside(client, project, &resolved, workspace.as_deref(), &request)
                    .map_err(|error| not_opened(project, &resolved, &request, error))
            })
            .await?;
        structured(with_alerts(json(&view)?, &alerts)?, false)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Arc, time::Duration};

    use rmcp::model::ErrorCode;
    use serde_json::Value;

    use super::*;
    use crate::{
        adapter::{
            client::{
                fake_service::FakeService,
                open::{attach_command, attach_hint},
                recording_workspace::RecordingWorkspace,
                served_service::ServedService,
            },
            mcp::test_support::{
                Fixture, content, fixture, fixture_with, refusal, register, runtime,
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
                &fixture.binary(),
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

    /// A registered fixture driving `served` in no terminal workspace.
    fn outside(served: &ServedService) -> Fixture {
        let fixture = fixture_with(FakeService::default().starting(served), None);
        register(&fixture);
        fixture
    }

    /// The failure `json` asks of `fixture`'s server, answered as a result.
    fn failed_open(fixture: &Fixture, json: Value) -> Value {
        let result = runtime().block_on(async { fixture.server().process_open(open(json)).await });
        let answer = result.unwrap_err().answer().unwrap();
        assert_eq!(answer.is_error, Some(true));
        answer.structured_content.unwrap()["error"].clone()
    }

    /// Outside a terminal workspace a running process is the agent's to
    /// answer, as `no-workspace` naming the attach the user can run: the
    /// pane's own command, with the project's root and `--` before the
    /// name, for the project the agent named too.
    #[test]
    fn process_open_outside_a_workspace_is_no_workspace_with_the_attach_hint() {
        for name in ["web", "-x", "$(id) x"] {
            let served = service(&[name], located());
            let fixture = outside(&served);
            let other = fixture.root().as_ref().join("frontend");
            runtime().block_on(async {
                content(
                    fixture
                        .server()
                        .process_add(Parameters(
                            serde_json::from_value(serde_json::json!({
                                "name": "web",
                                "command": "npm run dev",
                                "project": other,
                            }))
                            .unwrap(),
                        ))
                        .await,
                );
            });
            let other = ProjectRoot::try_new(other).unwrap();

            for (project, root) in [(None, fixture.root()), (Some(&other), &other)] {
                let mut json = serde_json::json!({"name": name});
                if let Some(project) = project {
                    json["project"] = serde_json::json!(project);
                }
                let failure = failed_open(&fixture, json);

                assert_eq!(failure["kind"], "no-workspace", "{name}");
                let message = failure["message"].as_str().unwrap();
                assert!(
                    message.starts_with("not inside a terminal workspace"),
                    "{message}"
                );
                let hint = attach_hint(root, &ProcessName::try_new(name).unwrap(), false);
                assert!(
                    message.contains(&format!("run {hint} in their own terminal")),
                    "{message}"
                );
                let root = root.as_ref().to_str().unwrap();
                assert_eq!(shlex::split(&hint).unwrap(), [
                    "muster",
                    "--project",
                    root,
                    "attach",
                    "--",
                    name
                ]);
            }
        }
    }

    /// Outside a terminal workspace the process is still resolved and
    /// located first: an unknown or stopped one gets its own answer, and a
    /// nul in a name never reaches a hint.
    #[test]
    fn process_open_outside_a_workspace_resolves_before_no_workspace() {
        for (located, typed, kind) in [
            (refused(ServiceErrorKind::NotRunning), "web", "not-running"),
            (located(), "nope", "unknown-process"),
            (located(), "x\u{0}; touch pwned", "unknown-process"),
        ] {
            let served = service(&["web"], located);
            let fixture = outside(&served);

            let failure = failed_open(&fixture, serde_json::json!({"name": typed}));

            assert_eq!(failure["kind"], kind, "{typed}");
            let message = failure["message"].as_str().unwrap();
            assert!(
                !message.contains("terminal") && !message.contains("attach"),
                "{message:?}"
            );
            if kind == "unknown-process" {
                assert_eq!(failure["candidates"], serde_json::json!(["web"]), "{typed}");
            }
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
        let hint = attach_hint(fixture.root(), &ProcessName::try_new("web").unwrap(), false);
        assert!(
            message.contains(&format!("run {hint} in their own terminal")),
            "{message}"
        );
        assert_eq!(workspace.requests().len(), 1);
    }

    /// Each open reads the binary anew, so the pane runs the binary as it
    /// is at that open, not as it was when the server started.
    #[test]
    fn each_open_reads_the_binary_anew() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);
        let first = fixture.binary();
        let second = first.with_file_name("muster-upgraded");
        fs::write(&second, "").unwrap();

        for binary in [&first, &second] {
            fixture.use_binary(binary.clone());
            runtime().block_on(async {
                content(
                    fixture
                        .server()
                        .process_open(open(serde_json::json!({"name": "web"})))
                        .await,
                )
            });
        }

        let programs: Vec<PathBuf> = workspace
            .requests()
            .iter()
            .map(|request| request.command().program().clone())
            .collect();
        assert_eq!(programs, [first, second]);
    }

    /// A binary removed since the server started, as Linux names one an
    /// upgrade replaced, fails the open as `workspace-failed` naming the
    /// path, and no pane is asked for.
    #[test]
    fn a_binary_gone_since_the_server_started_opens_no_pane() {
        let served = service(&["web"], located());
        let workspace = Arc::new(RecordingWorkspace::opening(OPENED));
        let fixture = inside(&served, &workspace);
        let removed = fixture.binary();
        fs::remove_file(&removed).unwrap();
        let replaced = PathBuf::from(format!("{} (deleted)", removed.display()));

        for gone in [removed, replaced] {
            fixture.use_binary(gone.clone());
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

            let failure = error.data.expect("the failure rides along");
            assert_eq!(failure["error"]["kind"], "workspace-failed");
            let message = failure["error"]["message"].as_str().unwrap();
            assert!(message.contains(&gone.display().to_string()), "{message}");
        }
        assert!(workspace.requests().is_empty());
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
