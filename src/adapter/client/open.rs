//! Opening a running process in a new pane of the terminal workspace the
//! caller runs in, attached to it, as both drivers do.

use std::{ffi::OsString, iter, path::Path};

use shlex::Quoter;

use super::{
    client_error::ClientError, open_request::OpenRequest, open_view::OpenView, process::locate,
    service_client::ServiceClient,
};
use crate::{
    constants::APP_NAME,
    domain::{
        port::TerminalWorkspace,
        project::Project,
        terminal_workspace::{PaneCommand, PaneRequest},
        value::{ProcessName, ProjectRoot, printable},
    },
};

/// The subcommand a pane runs to show the process.
const ATTACH_SUBCOMMAND: &str = "attach";
/// Names the project by its root, so a renamed project is never mismatched.
const PROJECT_FLAG: &str = "--project";
/// Makes the pane watch-only.
const READ_ONLY_FLAG: &str = "--read-only";
/// Ends the options, so a name starting with `-` is read as the name.
const END_OF_OPTIONS: &str = "--";

/// The argv a pane runs to attach, through `binary`, to the process `name`
/// of the project at `root`, watch-only when `read_only`.
pub fn attach_command(
    binary: &Path,
    root: &ProjectRoot,
    name: &ProcessName,
    read_only: bool,
) -> PaneCommand {
    let mut args: Vec<OsString> = vec![
        PROJECT_FLAG.into(),
        root.as_ref().clone().into_os_string(),
        ATTACH_SUBCOMMAND.into(),
    ];
    if read_only {
        args.push(READ_ONLY_FLAG.into());
    }
    args.extend([END_OF_OPTIONS.into(), name.as_ref().into()]);
    PaneCommand::builder()
        .program(binary.to_path_buf())
        .args(args)
        .build()
}

/// The command line a person runs in their own terminal to attach to the
/// process `name` of the project at `root`: the pane's [`attach_command`],
/// each word made [`printable`] and then shell-quoted.
pub fn attach_hint(root: &ProjectRoot, name: &ProcessName, read_only: bool) -> String {
    let command = attach_command(Path::new(APP_NAME), root, name, read_only);
    let words: Vec<String> = iter::once(command.program().as_os_str())
        .chain(command.args().iter().map(OsString::as_os_str))
        .map(|word| printable(&word.to_string_lossy()))
        .collect();
    // `printable` has escaped every nul, and a quoter allowing nul never fails.
    Quoter::new()
        .allow_nul(true)
        .join(words.iter().map(String::as_str))
        .unwrap_or_default()
}

/// Locates the process `name` of `project`, refusing one that is unknown or
/// not running before anything opens, then asks `workspace` for a pane
/// beside the caller's, in the project's folder, attached to it as
/// `request` says. A binary that no longer exists opens nothing, since the
/// pane could not run it.
///
/// # Errors
/// Returns the locate's [`ClientError`], [`ClientError::NoWorkspace`] for a
/// running process when there is no `workspace`, [`ClientError::BinaryGone`]
/// for a binary that no longer exists, or [`ClientError::Workspace`] when the
/// workspace did not open the pane.
pub fn open_beside(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessName,
    workspace: Option<&dyn TerminalWorkspace>,
    request: &OpenRequest,
) -> Result<OpenView, ClientError> {
    locate(client, project, name)?;
    let workspace = workspace.ok_or(ClientError::NoWorkspace)?;
    if !request.binary().is_file() {
        return Err(ClientError::BinaryGone(request.binary().clone()));
    }
    let root = project.root();
    let pane = PaneRequest::builder()
        .cwd(root.as_ref().clone())
        .command(attach_command(
            request.binary(),
            root,
            name,
            request.read_only(),
        ))
        .focus(request.focus())
        .direction(request.direction())
        .build();
    let opened = workspace.open_beside(&pane)?;
    Ok(OpenView::builder()
        .project(project.name().clone())
        .process(name.clone())
        .pane(opened.pane().clone())
        .read_only(request.read_only())
        .build())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::{
            client::{
                FailureKind, recording_workspace::RecordingWorkspace,
                served_service::ServedService, this_binary,
            },
            service::{
                Located, ServiceErrorBody, ServiceErrorKind, ServiceRequest, ServiceResponse,
            },
        },
        domain::{
            terminal_workspace::{PaneFocus, SplitDirection, WorkspaceError},
            value::ProjectName,
        },
    };

    /// A muster binary that does not exist.
    const BINARY: &str = "/opt/muster/bin/muster";
    /// The project's folder, with a space in it.
    const ROOT: &str = "/home/me/my shop";
    /// The pane a workspace opens.
    const OPENED: &str = "w1:p4";

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

    /// The arguments of `command`, as text.
    fn args(command: &PaneCommand) -> Vec<String> {
        command
            .args()
            .iter()
            .map(|arg| arg.to_str().unwrap().to_string())
            .collect()
    }

    /// An interactive request that takes the focus, split as `direction`,
    /// running this test binary, which exists.
    fn request(read_only: bool, direction: Option<SplitDirection>) -> OpenRequest {
        OpenRequest::builder()
            .binary(this_binary())
            .read_only(read_only)
            .focus(PaneFocus::Take)
            .direction(direction)
            .build()
    }

    /// A service that locates every process asked for.
    fn locating() -> ServedService {
        ServedService::answering(ServiceResponse::Located(
            Located::builder()
                .socket(PathBuf::from("/run/holder.sock"))
                .log(PathBuf::from("/state/web.log"))
                .build(),
        ))
    }

    /// A service refusing every request with `kind`.
    fn refusing(kind: ServiceErrorKind) -> ServedService {
        ServedService::answering(ServiceResponse::Error(
            ServiceErrorBody::builder()
                .kind(kind)
                .message("refused".to_string())
                .build(),
        ))
    }

    /// The command names the project by its root, and ends the options
    /// before the name, so a name starting with `-` is never read as a flag.
    #[test]
    fn the_attach_command_names_the_root_and_ends_options_before_the_name() {
        let command = attach_command(Path::new(BINARY), shop().root(), &name("-x"), false);

        assert_eq!(command.program(), Path::new(BINARY));
        assert_eq!(args(&command), ["--project", ROOT, "attach", "--", "-x"]);
    }

    /// A watch-only pane attaches read-only.
    #[test]
    fn read_only_adds_the_read_only_flag() {
        let command = attach_command(Path::new(BINARY), shop().root(), &name("web"), true);

        assert_eq!(args(&command), [
            "--project",
            ROOT,
            "attach",
            "--read-only",
            "--",
            "web"
        ]);
    }

    /// The hint is the pane's own command run as `muster`: the project by
    /// its root and `--` before a name starting with `-`, so it pastes back
    /// as the same words whatever the folder or name hold.
    #[test]
    fn the_attach_hint_is_the_pane_command_quoted() {
        let other = ProjectRoot::try_new(PathBuf::from("/srv/other's app")).unwrap();

        assert_eq!(
            attach_hint(shop().root(), &name("-x"), false),
            "muster --project '/home/me/my shop' attach -- -x"
        );
        assert_eq!(
            attach_hint(&other, &name("dev server"), true),
            "muster --project \"/srv/other's app\" attach --read-only -- 'dev server'"
        );
        assert_eq!(
            shlex::split(&attach_hint(&other, &name("$(id)"), false)).unwrap(),
            [
                "muster",
                "--project",
                "/srv/other's app",
                "attach",
                "--",
                "$(id)"
            ]
        );
    }

    /// A nul or a control character is made printable before quoting, so
    /// it can never leave a word unquoted or reach the terminal.
    #[test]
    fn the_attach_hint_quotes_a_nul_and_a_control_character() {
        let root = ProjectRoot::try_new(PathBuf::from("/srv/x\0; touch pwned\u{1b}[2J")).unwrap();

        let hint = attach_hint(&root, &name("web"), false);

        assert_eq!(
            hint,
            r#"muster --project "/srv/x\\u{0}; touch pwned\\u{1b}[2J" attach -- web"#
        );
        assert_eq!(
            shlex::split(&hint).unwrap()[2],
            r"/srv/x\u{0}; touch pwned\u{1b}[2J"
        );
    }

    /// Opening locates the process, then asks for one pane in the project's
    /// folder running attach, and answers with the pane it opened.
    #[test]
    fn open_asks_for_a_pane_in_the_project_root_running_attach() {
        let served = locating();
        let workspace = RecordingWorkspace::opening(OPENED);

        let view = open_beside(
            &served.connector()(),
            &shop(),
            &name("web"),
            Some(&workspace),
            &request(true, Some(SplitDirection::Down)),
        )
        .unwrap();

        let expected = PaneRequest::builder()
            .cwd(PathBuf::from(ROOT))
            .command(attach_command(
                &this_binary(),
                shop().root(),
                &name("web"),
                true,
            ))
            .focus(PaneFocus::Take)
            .direction(Some(SplitDirection::Down))
            .build();
        assert_eq!(workspace.requests(), [expected]);
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({"project": "shop", "process": "web", "pane": OPENED, "read_only": true})
        );
        assert!(served.asked().iter().any(|asked| matches!(
            asked,
            ServiceRequest::Locate { name, .. } if name.as_ref() == "web"
        )));
    }

    /// A process the service does not know or does not run fails as attach
    /// would, and no pane is asked for.
    #[test]
    fn an_unknown_or_stopped_process_opens_no_pane() {
        for (kind, failure) in [
            (
                ServiceErrorKind::UnknownProcess,
                FailureKind::UnknownProcess,
            ),
            (ServiceErrorKind::NotRunning, FailureKind::NotRunning),
        ] {
            let served = refusing(kind);
            let workspace = RecordingWorkspace::opening(OPENED);

            let error = open_beside(
                &served.connector()(),
                &shop(),
                &name("web"),
                Some(&workspace),
                &request(false, None),
            )
            .unwrap_err();

            assert_eq!(error.kind(), failure);
            assert!(workspace.requests().is_empty());
        }
    }

    /// Without a workspace, a running process is `no-workspace` once it is
    /// located, and an unknown or stopped one fails as the locate says.
    #[test]
    fn without_a_workspace_a_running_process_is_no_workspace_after_the_locate() {
        let served = locating();

        let error = open_beside(
            &served.connector()(),
            &shop(),
            &name("web"),
            None,
            &request(false, None),
        )
        .unwrap_err();

        assert_eq!(error.kind(), FailureKind::NoWorkspace);
        assert!(served.asked().iter().any(|asked| matches!(
            asked,
            ServiceRequest::Locate { name, .. } if name.as_ref() == "web"
        )));
        let stopped = open_beside(
            &refusing(ServiceErrorKind::NotRunning).connector()(),
            &shop(),
            &name("web"),
            None,
            &request(false, None),
        )
        .unwrap_err();
        assert_eq!(stopped.kind(), FailureKind::NotRunning);
    }

    /// A workspace that does not open the pane fails the open as
    /// `workspace-failed`, carrying the workspace's own words.
    #[test]
    fn a_workspace_failure_is_workspace_failed() {
        let served = locating();
        let refusal = WorkspaceError::Refused {
            workspace: "tiles".to_string(),
            action: "split a pane".to_string(),
            code: "pane_not_found".to_string(),
            message: "no such pane".to_string(),
        };
        let workspace = RecordingWorkspace::failing(refusal.clone());

        let error = open_beside(
            &served.connector()(),
            &shop(),
            &name("web"),
            Some(&workspace),
            &request(false, None),
        )
        .unwrap_err();

        assert_eq!(error.kind(), FailureKind::WorkspaceFailed);
        assert_eq!(error.to_string(), refusal.to_string());
        assert_eq!(workspace.requests().len(), 1);
    }

    /// A binary that no longer exists, removed or named by Linux as
    /// replaced, fails as `workspace-failed` naming it, and no pane is
    /// asked for.
    #[test]
    fn a_binary_that_no_longer_exists_opens_no_pane() {
        let replaced = PathBuf::from(format!("{} (deleted)", this_binary().display()));
        for gone in [PathBuf::from(BINARY), replaced] {
            let served = locating();
            let workspace = RecordingWorkspace::opening(OPENED);
            let asked = OpenRequest::builder()
                .binary(gone.clone())
                .read_only(false)
                .focus(PaneFocus::Take)
                .direction(None)
                .build();

            let error = open_beside(
                &served.connector()(),
                &shop(),
                &name("web"),
                Some(&workspace),
                &asked,
            )
            .unwrap_err();

            assert!(matches!(&error, ClientError::BinaryGone(path) if *path == gone));
            assert_eq!(error.kind(), FailureKind::WorkspaceFailed);
            assert!(
                error.to_string().contains(&gone.display().to_string()),
                "{error}"
            );
            assert!(workspace.requests().is_empty());
        }
    }
}
