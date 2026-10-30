//! `muster open` inside a terminal workspace: the running process chosen or
//! named, shown in a new pane beside this one.

use std::path::Path;

use super::{
    error::CliError, open_args::OpenArgs, presenter::Presenter, process::running_named_or_chosen,
};
use crate::{
    adapter::client::{ClientError, OpenRequest, OpenView, ServiceClient, open::open_beside},
    domain::{port::TerminalWorkspace, project::Project, terminal_workspace::PaneFocus},
};

/// Opens the running process of `project` that `args` names, or the one
/// chosen through `presenter`, in a pane of `workspace` beside this one that
/// takes the focus and runs `binary` to attach, as `args` asks.
///
/// # Errors
/// Returns the [`CliError`] of a process that cannot be found or chosen, or
/// is not running, before any pane is asked for, or
/// [`CliError::OpenFailed`] when the workspace did not open the pane.
pub fn open_here(
    client: &ServiceClient,
    project: &Project,
    args: &OpenArgs,
    workspace: &dyn TerminalWorkspace,
    binary: &Path,
    presenter: Presenter,
) -> Result<OpenView, CliError> {
    let name = running_named_or_chosen(client, project, args.name().as_ref(), presenter)?;
    let request = OpenRequest::builder()
        .binary(binary.to_path_buf())
        .read_only(args.read_only())
        .focus(PaneFocus::Take)
        .direction(args.direction())
        .build();
    open_beside(client, project, &name, workspace, &request).map_err(|error| match error {
        ClientError::Workspace(source) => CliError::OpenFailed { name, source },
        other => other.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::{iter, path::PathBuf, time::Duration};

    use clap::Parser;

    use super::*;
    use crate::{
        adapter::{
            cli::{
                args::Args, cli_exit::CliExit, command::Command, dispatch::parse_failure, output,
            },
            client::{
                FailureKind, open::attach_command, recording_workspace::RecordingWorkspace,
                served_service::ServedService,
            },
            service::{
                Located, OccupancyStatus, ProcessStatus, ProjectStatus, ServiceErrorBody,
                ServiceErrorKind, ServiceRequest, ServiceResponse,
            },
        },
        constants::APP_NAME,
        domain::{
            process::ProcessState,
            terminal_workspace::{PaneRequest, SplitDirection, WorkspaceError},
            value::{IdleStop, PositiveDuration, ProcessName, ProjectName, ProjectRoot},
        },
    };

    /// The muster binary each pane runs.
    const BINARY: &str = "/opt/muster/bin/muster";
    /// The project's folder, with a space in it.
    const ROOT: &str = "/home/me/my shop";
    /// The pane the workspace opens.
    const OPENED: &str = "w1:p4";
    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    /// The project `shop` at [`ROOT`].
    fn shop() -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from(ROOT)).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .build()
    }

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// The arguments of `muster open` followed by `words`.
    fn open_args(words: &[&str]) -> OpenArgs {
        let args = Args::try_parse_from([APP_NAME, "open"].iter().chain(words)).unwrap();
        match args.into_command() {
            Some(Command::Open(open)) => open,
            _ => panic!("open parses"),
        }
    }

    /// `named` listed in `state`.
    fn listed(named: &str, state: ProcessState) -> ProcessStatus {
        ProcessStatus::builder()
            .name(name(named))
            .state(state)
            .pid(None)
            .uptime(None)
            .restarts(0)
            .usage(None)
            .attached(0)
            .last_exit(None)
            .stop_grace(Duration::from_secs(5))
            .build()
    }

    /// The service's listing of `shop` with `processes`.
    fn listing(processes: Vec<ProcessStatus>) -> ServiceResponse {
        ServiceResponse::Status(vec![
            ProjectStatus::builder()
                .project(shop())
                .occupancy(
                    OccupancyStatus::builder()
                        .shells(0)
                        .idle_stop(IdleStop::After(
                            PositiveDuration::try_new(Duration::from_secs(1800)).unwrap(),
                        ))
                        .stops_in(None)
                        .build(),
                )
                .processes(processes)
                .build(),
        ])
    }

    /// A service listing `api` running and `web` exited, and answering every
    /// other request with `then`.
    fn serving(then: ServiceResponse) -> ServedService {
        let listing = listing(vec![
            listed("api", ProcessState::Running),
            listed("web", ProcessState::Exited),
        ]);
        ServedService::responding(move |request| match request {
            ServiceRequest::Ps { .. } => listing.clone(),
            _ => then.clone(),
        })
    }

    /// The answer of a service that found the holder.
    fn located() -> ServiceResponse {
        ServiceResponse::Located(
            Located::builder()
                .socket(PathBuf::from("/run/holder.sock"))
                .log(PathBuf::from("/state/api.log"))
                .build(),
        )
    }

    /// Opens what `words` ask for in `workspace` through `served`.
    fn open(
        served: &ServedService,
        workspace: &RecordingWorkspace,
        words: &[&str],
    ) -> Result<OpenView, CliError> {
        open_here(
            &served.connector()(),
            &shop(),
            &open_args(words),
            workspace,
            Path::new(BINARY),
            Presenter::Plain,
        )
    }

    /// The workspace's refusal to split a pane.
    fn refusal() -> WorkspaceError {
        WorkspaceError::Refused {
            workspace: "tiles".to_string(),
            action: "split a pane".to_string(),
            code: "pane_not_found".to_string(),
            message: "no such pane".to_string(),
        }
    }

    /// A named, running process opens in one focused pane in the project's
    /// folder, attached to it by name, split as the workspace picks.
    #[test]
    fn open_inside_a_workspace_asks_for_a_focused_pane_attached_to_the_name() {
        let served = serving(located());
        let workspace = RecordingWorkspace::opening(OPENED);

        let view = open(&served, &workspace, &["api"]).unwrap();

        let expected = PaneRequest::builder()
            .cwd(PathBuf::from(ROOT))
            .command(attach_command(
                Path::new(BINARY),
                shop().root(),
                &name("api"),
                false,
            ))
            .focus(PaneFocus::Take)
            .direction(None)
            .build();
        assert_eq!(workspace.requests(), [expected]);
        assert_eq!(view.pane().as_ref(), OPENED);
        assert!(served.asked().iter().any(|asked| matches!(
            asked,
            ServiceRequest::Locate { name, .. } if name.as_ref() == "api"
        )));
    }

    /// `--read-only` and `--direction` reach the pane asked for.
    #[test]
    fn open_read_only_and_direction_reach_the_request() {
        let served = serving(located());
        let workspace = RecordingWorkspace::opening(OPENED);

        let view = open(&served, &workspace, &[
            "api",
            "--read-only",
            "--direction",
            "down",
        ])
        .unwrap();

        let [request] = workspace.requests().try_into().unwrap();
        assert_eq!(request.direction(), Some(SplitDirection::Down));
        assert_eq!(
            request.command(),
            &attach_command(Path::new(BINARY), shop().root(), &name("api"), true)
        );
        assert!(view.read_only());
    }

    /// `--json` prints the project, the process, the pane and whether it
    /// only watches.
    #[test]
    fn open_json_prints_project_process_pane_and_read_only() {
        let served = serving(located());
        let workspace = RecordingWorkspace::opening(OPENED);

        let view = open(&served, &workspace, &["api", "--json"]).unwrap();

        insta::assert_snapshot!("open_json", output::json(&view).unwrap());
    }

    /// A process that is not running fails as attach fails, and no pane is
    /// asked for.
    #[test]
    fn open_of_a_stopped_process_opens_no_pane_and_exits_like_attach() {
        let served = serving(ServiceResponse::Error(
            ServiceErrorBody::builder()
                .kind(ServiceErrorKind::NotRunning)
                .message("'web' is not running".to_string())
                .build(),
        ));
        let workspace = RecordingWorkspace::opening(OPENED);

        let error = open(&served, &workspace, &["web"]).unwrap_err();

        assert_eq!(error.kind(), FailureKind::NotRunning);
        assert!(matches!(error, CliError::Client(_)), "{error:?}");
        assert!(workspace.requests().is_empty());
    }

    /// A name no process has, or one two processes share but for their
    /// encoding, fails before the holder is looked for, and no pane is
    /// asked for.
    #[test]
    fn an_unknown_or_ambiguous_name_opens_no_pane() {
        for (processes, typed, kind) in [
            (vec!["api"], "worker", FailureKind::UnknownProcess),
            (
                vec![CAFE_DECOMPOSED, CAFE_DECOMPOSED],
                CAFE_COMPOSED,
                FailureKind::Ambiguous,
            ),
        ] {
            let served = ServedService::answering(listing(
                processes
                    .into_iter()
                    .map(|named| listed(named, ProcessState::Running))
                    .collect(),
            ));
            let workspace = RecordingWorkspace::opening(OPENED);

            let error = open(&served, &workspace, &[typed]).unwrap_err();

            assert_eq!(error.kind(), kind, "{typed}");
            assert!(workspace.requests().is_empty());
            assert!(
                !served
                    .asked()
                    .iter()
                    .any(|asked| matches!(asked, ServiceRequest::Locate { .. }))
            );
        }
    }

    /// A workspace that does not open the pane fails the command with exit
    /// 1, in its own words and naming the attach that shows the process
    /// here instead, in plain text and in JSON.
    #[test]
    fn a_workspace_failure_exits_1_naming_the_attach_hint() {
        let served = serving(located());
        let workspace = RecordingWorkspace::failing(refusal());

        let error = open(&served, &workspace, &["api"]).unwrap_err();

        assert_eq!(error.kind(), FailureKind::WorkspaceFailed);
        assert_eq!(CliExit::from(error.kind()), CliExit::ActionFailed);
        insta::assert_snapshot!("open_workspace_failed", error.to_string());
        insta::assert_snapshot!("open_workspace_failed_json", output::failure_json(&error));
    }

    /// The argv a pane runs parses, through the real command line, as an
    /// attach to the named process of the project at its root, so a name
    /// starting with `-` is read as the name.
    #[test]
    fn the_pane_command_parses_as_attach() {
        for read_only in [false, true] {
            let command = attach_command(Path::new(BINARY), shop().root(), &name("-x"), read_only);

            let args = Args::try_parse_from(
                iter::once(command.program().as_os_str().to_os_string())
                    .chain(command.args().iter().cloned()),
            )
            .unwrap();

            assert_eq!(args.project().as_deref(), Some(ROOT));
            match args.into_command() {
                Some(Command::Attach {
                    name: Some(name),
                    read_only: parsed,
                    json: false,
                }) => {
                    assert_eq!(name.as_ref(), "-x");
                    assert_eq!(parsed, read_only);
                },
                _ => panic!("the pane command parses as attach"),
            }
        }
    }

    /// A direction that is not one of the split directions is refused as an
    /// invalid value, listing them.
    #[test]
    fn a_bad_direction_is_a_usage_error() {
        let Err(error) = Args::try_parse_from([APP_NAME, "open", "api", "--direction", "up"])
        else {
            panic!("an unknown direction parses");
        };

        let failure = parse_failure(&error);
        assert_eq!(failure.kind(), FailureKind::InvalidInput);
        for direction in ["right", "down"] {
            assert!(failure.to_string().contains(direction), "{failure}");
        }
    }
}
