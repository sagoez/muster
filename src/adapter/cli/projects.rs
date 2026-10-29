//! `muster projects`: the registered projects, and removing one.

use std::path::Path;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    output::{COLUMN_GAP, action_rows, table_lines},
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::client::{
        ActionReport, ProjectIdentityView, ServiceClient, SettleWatch, process::down,
    },
    domain::{
        config::ConfigError,
        port::{LocalState, StateChangeNotifier},
        project::Project,
    },
};

/// Title of the `projects` reports.
const PROJECTS_TITLE: &str = "muster projects";
/// Marks the project containing the current folder.
const CURRENT_MARKER: &str = "* ";
/// Aligns the other projects with the marked one.
const OTHER_INDENT: &str = "  ";
/// One process, counted.
const PROCESS: &str = "process";
/// Several processes, counted.
const PROCESSES: &str = "processes";
/// Said when no project is registered.
const NO_PROJECTS: &str =
    "no registered projects; `muster add <name> -- <command>` in a folder starts one";
/// Opens the reason a project's processes do not read.
const INVALID: &str = "unreadable: ";
/// Said when a removed project's processes could not be stopped first.
const NOTHING_STOPPED: &str = "its stored processes did not read, so none were stopped first";
/// Opens the line saying a project was removed.
const REMOVED: &str = "removed project";
/// The question `projects rm` asks at a terminal.
pub const REMOVE_QUESTION: &str =
    "Remove this project, its processes and its shared notes, todos and values?";
/// What the plain layout says it could not ask.
pub const REMOVE_ASKED: &str = "whether to remove the project";

/// One registered project, as `projects --json` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProjectListing {
    /// The project.
    #[serde(flatten)]
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// How many processes it has, or none when its stored processes do not
    /// read.
    #[getset(get_copy = "pub")]
    processes: Option<usize>,
    /// Why its stored processes do not read, when they do not.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    invalid: Option<String>,
    /// Whether it contains the current folder.
    #[getset(get_copy = "pub")]
    current: bool,
}

/// What `projects rm` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProjectRemoval {
    /// The project removed.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// Whether it was still registered when removed.
    #[getset(get_copy = "pub")]
    removed: bool,
    /// What became of its processes as they were stopped, or none when its
    /// stored processes did not read and the service could not stop them
    /// first.
    #[getset(get = "pub")]
    stopped: Option<ActionReport>,
}

impl ProjectRemoval {
    /// Whether a process it stopped did not end up stopped.
    pub fn failed(&self) -> bool {
        self.stopped.as_ref().is_some_and(ActionReport::failed)
    }
}

/// Every registered project, the one containing `cwd` marked current. A
/// project whose stored processes do not read is listed with the reason, so
/// one corrupt row never hides the others.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn list(store: &dyn LocalState, cwd: &Path) -> Result<Vec<ProjectListing>, CliError> {
    let current = store.project_containing(cwd)?;
    store
        .projects()?
        .iter()
        .map(|project| {
            let (processes, invalid) = match store.stack(project.root()) {
                Ok(stack) => (Some(stack.map_or(0, |stack| stack.processes().len())), None),
                Err(error @ ConfigError::CorruptRow { .. }) => (None, Some(error.to_string())),
                Err(error) => return Err(error.into()),
            };
            Ok(ProjectListing::builder()
                .project(ProjectIdentityView::from(project))
                .processes(processes)
                .invalid(invalid)
                .current(current.as_ref() == Some(project))
                .build())
        })
        .collect()
}

/// The human list of `listings`: `name  folder  N processes`, aligned,
/// the current project marked.
pub fn list_rows(listings: &[ProjectListing]) -> Report {
    if listings.is_empty() {
        return Report::new(PROJECTS_TITLE, vec![Row::unlabeled(
            RowKind::Hint,
            NO_PROJECTS,
        )]);
    }
    let cells: Vec<Vec<String>> = listings
        .iter()
        .map(|listing| {
            let contents = match (listing.processes(), listing.invalid()) {
                (Some(1), _) => format!("1 {PROCESS}"),
                (Some(count), _) => format!("{count} {PROCESSES}"),
                (None, reason) => format!("{INVALID}{}", reason.as_deref().unwrap_or_default()),
            };
            vec![
                listing.project().name().to_string(),
                listing.project().root().display().to_string(),
                contents,
            ]
        })
        .collect();
    let rows = listings
        .iter()
        .zip(table_lines(&cells))
        .map(|(listing, line)| {
            let marker = if listing.current() {
                CURRENT_MARKER
            } else {
                OTHER_INDENT
            };
            Row::unlabeled(RowKind::Plain, format!("{marker}{line}"))
        })
        .collect();
    Report::new(PROJECTS_TITLE, rows)
}

/// Stops every process of `project` through `client`, then forgets the
/// project with its processes, environment and coordination state, and tells
/// the service. `watch` hears what is still stopping.
///
/// # Errors
/// Returns the [`CliError`] of a service that cannot be reached or a store
/// that cannot be written.
pub fn remove(
    client: &ServiceClient,
    store: &dyn LocalState,
    notifier: &dyn StateChangeNotifier,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ProjectRemoval, CliError> {
    let stopped = match down(client, project, watch) {
        Ok(stopped) => Some(stopped),
        Err(_)
            if matches!(
                store.stack(project.root()),
                Err(ConfigError::CorruptRow { .. })
            ) =>
        {
            None
        },
        Err(error) => return Err(error.into()),
    };
    let removed = store.remove_project(project.root())?;
    notifier.stack_changed(project.root());
    Ok(ProjectRemoval::builder()
        .project(ProjectIdentityView::from(project))
        .removed(removed)
        .stopped(stopped)
        .build())
}

/// The human report of `removal`: what became of the processes, then the
/// project forgotten.
pub fn removal_rows(removal: &ProjectRemoval) -> Report {
    let mut rows: Vec<Row> = removal.stopped().as_ref().map_or_else(
        || vec![Row::unlabeled(RowKind::Hint, NOTHING_STOPPED)],
        |stopped| action_rows(stopped).rows().clone(),
    );
    rows.push(Row::unlabeled(
        RowKind::Ok,
        format!(
            "{REMOVED} {}{COLUMN_GAP}{}",
            removal.project().name(),
            removal.project().root().display()
        ),
    ));
    Report::new(PROJECTS_TITLE, rows).with_failure(removal.failed())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::cli::report::plain,
        domain::value::{ProjectName, ProjectRoot},
    };

    /// A listing of `name` at `root` with `processes`, `current` or not.
    fn listing(name: &str, root: &str, processes: usize, current: bool) -> ProjectListing {
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from(root)).unwrap())
            .name(ProjectName::try_new(name).unwrap())
            .build();
        ProjectListing::builder()
            .project(ProjectIdentityView::from(&project))
            .processes(Some(processes))
            .current(current)
            .build()
    }

    /// A project whose processes do not read is listed with the reason.
    #[test]
    fn an_unreadable_project_is_listed_with_its_reason() {
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/srv/blog")).unwrap())
            .name(ProjectName::try_new("blog").unwrap())
            .build();
        let broken = ProjectListing::builder()
            .project(ProjectIdentityView::from(&project))
            .processes(None)
            .invalid(Some("stored stop grace is invalid".to_string()))
            .current(false)
            .build();

        assert_eq!(
            plain(&list_rows(&[
                listing("shop", "/home/me/shop", 3, true),
                broken.clone()
            ])),
            "* shop  /home/me/shop  3 processes\n  blog  /srv/blog      unreadable: stored stop \
             grace is invalid"
        );
        assert_eq!(
            serde_json::to_value(broken).unwrap(),
            serde_json::json!({"name": "blog", "root": "/srv/blog", "processes": null, "current": false, "invalid": "stored stop grace is invalid"})
        );
    }

    /// Each project is one aligned line with its folder and count, the
    /// current one marked.
    #[test]
    fn projects_are_listed_aligned_with_the_current_one_marked() {
        let report = list_rows(&[
            listing("shop", "/home/me/shop", 3, true),
            listing("blog-2", "/srv/blog", 1, false),
        ]);

        assert_eq!(
            plain(&report),
            "* shop    /home/me/shop  3 processes\n  blog-2  /srv/blog      1 process"
        );
        assert!(plain(&list_rows(&[])).contains("muster add"));
    }

    /// The JSON names, roots, counts and marks each project.
    #[test]
    fn a_listing_reads_as_json() {
        assert_eq!(
            serde_json::to_value(listing("shop", "/home/me/shop", 3, true)).unwrap(),
            serde_json::json!({"name": "shop", "root": "/home/me/shop", "processes": 3, "current": true})
        );
    }
}
