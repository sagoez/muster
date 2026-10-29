//! `muster add`: a new process in a project, registering the folder first
//! when it is not one. Nothing is started.

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    args::AddArgs, command_words::command_string, error::CliError, invocation::Invocation,
    output::project_title, report::Report, row::Row, row_kind::RowKind,
    working_dir::stored_working_dir,
};
use crate::{
    adapter::client::{DefinitionView, ProjectIdentityView},
    domain::{
        config::{Environment, ProcessDraft},
        coordination::Author,
        value::{Description, ProcessName},
    },
};

/// The command word of `muster add`.
const ADD_WORD: &str = "add";
/// Opens the line naming the added process.
const ADDED: &str = "added";
/// Opens the line naming a newly registered project.
const REGISTERED: &str = "registered project";
/// Between a name and what follows it on a line.
const GAP: &str = "  ";
/// How a process that autostarts gets started.
const AUTOSTART_HINT: &str = "autostart on: `muster up` starts it";
/// The command that starts one process, before its name.
const START_COMMAND: &str = "muster start";
/// Follows the start command in a hint.
const STARTS_IT: &str = "starts it";

/// What `muster add` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct AddView {
    /// The project it was added to.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// Whether the folder was registered as a project by this add.
    #[getset(get_copy = "pub")]
    registered: bool,
    /// The process as stored, defaults filled in.
    #[getset(get = "pub")]
    process: DefinitionView,
}

/// Adds the process `args` defines to the project the invocation resolves
/// to, registering that folder when it is not a project yet.
///
/// # Errors
/// Returns [`CliError::ProcessNameTaken`] for a name the project already
/// has, or the [`CliError`] of a name, command, folder or variable that is
/// not usable.
pub fn add(invocation: &Invocation<'_>, args: AddArgs) -> Result<AddView, CliError> {
    let store = invocation.store()?;
    let target = invocation.editing_target(store.as_ref())?;
    let root = target.root().clone();
    let name =
        ProcessName::try_new(args.name.clone()).map_err(|_| CliError::InvalidName(args.name))?;
    let command = command_string(&args.command)?;
    let working_dir = match args.cwd {
        Some(dir) => stored_working_dir(&root, &invocation.in_cwd(&dir)?)?,
        None => None,
    };
    let description = args
        .description
        .map(|text| Description::try_new(text).map_err(|_| CliError::EmptyDescription))
        .transpose()?;
    let environment: Environment = args.flags.environment()?.into_iter().collect();
    let draft = ProcessDraft::builder()
        .name(name)
        .command(command)
        .working_dir(working_dir)
        .description(description)
        .environment(environment)
        .autostart(args.flags.autostart())
        .restart(args.flags.restart)
        .stop_signal(args.flags.stop_signal)
        .stop_grace(args.flags.stop_grace)
        .ready(args.flags.ready()?)
        .build();
    let (registration, spec) = invocation
        .editor(store.as_ref())
        .add(&root, draft, &Author::human())
        .map_err(|error| CliError::adding(error, target.name(), ""))?;
    Ok(AddView::builder()
        .project(ProjectIdentityView::from(registration.project()))
        .registered(*registration.newly_registered())
        .process(DefinitionView::from(&spec))
        .build())
}

/// The human report of `view`: the process added, the project registered
/// when it was new, and how to start it.
pub fn add_rows(view: &AddView) -> Report {
    let process = view.process();
    let mut rows = vec![Row::unlabeled(
        RowKind::Ok,
        format!("{ADDED} {}{GAP}{}", process.name(), process.command()),
    )];
    if view.registered() {
        rows.push(Row::unlabeled(
            RowKind::Plain,
            format!(
                "{REGISTERED} {}{GAP}{}",
                view.project().name(),
                view.project().root().display()
            ),
        ));
    }
    rows.push(Row::unlabeled(RowKind::Dim, start_hint(process)));
    Report::new(project_title(ADD_WORD, view.project().name()), rows)
}

/// How the process `process` gets started.
fn start_hint(process: &DefinitionView) -> String {
    if process.autostart() {
        AUTOSTART_HINT.to_string()
    } else {
        format!(
            "`{START_COMMAND} {}` {STARTS_IT}",
            process.name().shell_quoted()
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::{
        adapter::cli::report::plain,
        domain::{
            config::ProcessSpec,
            process::{RestartPolicy, StopPolicy, StopSignal},
            project::Project,
            value::{CommandLine, ProjectName, ProjectRoot, StopGrace},
        },
    };

    /// What adding `web` to a new project `shop` reports.
    fn view(autostart: bool, registered: bool) -> AddView {
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .environment(Environment::empty())
            .autostart(autostart)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build();
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .build();
        AddView::builder()
            .project(ProjectIdentityView::from(&project))
            .registered(registered)
            .process(DefinitionView::from(&spec))
            .build()
    }

    /// The first add registers the project and says so; a later one does
    /// not. The hint follows autostart.
    #[test]
    fn an_add_reports_the_process_the_project_and_how_to_start_it() {
        let first = add_rows(&view(false, true));
        assert_eq!(first.title(), "muster add · shop");
        assert_eq!(
            plain(&first),
            "added web  npm run dev\nregistered project shop  /home/me/shop\n`muster start 'web'` starts it"
        );
        assert_eq!(
            plain(&add_rows(&view(true, false))),
            "added web  npm run dev\nautostart on: `muster up` starts it"
        );
    }

    /// The JSON names the project, whether it was registered, and the
    /// definition.
    #[test]
    fn an_add_reads_as_documented_json() {
        let value = serde_json::to_value(view(false, true)).unwrap();

        assert_eq!(
            value["project"],
            serde_json::json!({"name": "shop", "root": "/home/me/shop"})
        );
        assert_eq!(value["registered"], true);
        assert_eq!(value["process"]["stop"]["grace"], 5.0);
    }
}
