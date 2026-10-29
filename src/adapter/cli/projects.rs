//! `muster projects`: the registered projects, and removing one.

use std::path::Path;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthStr;

use super::{
    action_report::ActionReport,
    error::CliError,
    output::action_rows,
    process::down,
    project_identity_view::ProjectIdentityView,
    report::{Report, Row, RowKind},
    running_service_notifier::RunningServiceNotifier,
    service_client::ServiceClient,
    settle_watch::SettleWatch,
};
use crate::{
    adapter::store::SqliteStore,
    domain::{
        port::{ProjectStore, StackStore, StateChangeNotifier},
        project::Project,
    },
};

/// Title of the `projects` reports.
const PROJECTS_TITLE: &str = "muster projects";
/// Marks the project containing the current folder.
const CURRENT_MARKER: &str = "* ";
/// Aligns the other projects with the marked one.
const OTHER_INDENT: &str = "  ";
/// Between the columns of a project's line.
const COLUMN_GAP: &str = "  ";
/// One process, counted.
const PROCESS: &str = "process";
/// Several processes, counted.
const PROCESSES: &str = "processes";
/// Said when no project is registered.
const NO_PROJECTS: &str =
    "no registered projects; `muster add <name> -- <command>` in a folder starts one";
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
    /// How many processes it has.
    #[getset(get_copy = "pub")]
    processes: usize,
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
    /// What became of its processes as they were stopped.
    #[getset(get = "pub")]
    stopped: ActionReport,
}

/// Every registered project, the one containing `cwd` marked current.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn list(store: &SqliteStore, cwd: &Path) -> Result<Vec<ProjectListing>, CliError> {
    let current = store.project_containing(cwd)?;
    store
        .projects()?
        .iter()
        .map(|project| {
            let processes = store
                .stack(project.root())?
                .map_or(0, |stack| stack.processes().len());
            Ok(ProjectListing::builder()
                .project(ProjectIdentityView::from(project))
                .processes(processes)
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
    let width = |column: fn(&ProjectListing) -> String| {
        listings
            .iter()
            .map(|listing| UnicodeWidthStr::width(column(listing).as_str()))
            .max()
            .unwrap_or(0)
    };
    let name = |listing: &ProjectListing| listing.project().name().to_string();
    let root = |listing: &ProjectListing| listing.project().root().display().to_string();
    let (name_width, root_width) = (width(name), width(root));
    let rows = listings
        .iter()
        .map(|listing| {
            let marker = if listing.current() {
                CURRENT_MARKER
            } else {
                OTHER_INDENT
            };
            let count = listing.processes();
            let noun = if count == 1 { PROCESS } else { PROCESSES };
            Row::unlabeled(
                RowKind::Plain,
                format!(
                    "{marker}{}{COLUMN_GAP}{}{COLUMN_GAP}{count} {noun}",
                    pad(&name(listing), name_width),
                    pad(&root(listing), root_width),
                ),
            )
        })
        .collect();
    Report::new(PROJECTS_TITLE, rows)
}

/// `text` padded with spaces to the display width `width`.
fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(UnicodeWidthStr::width(text));
    format!("{text}{}", " ".repeat(fill))
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
    store: &SqliteStore,
    project: &Project,
    watch: &mut dyn SettleWatch,
) -> Result<ProjectRemoval, CliError> {
    let stopped = down(client, project, watch)?;
    let removed = store.remove_project(project.root())?;
    RunningServiceNotifier.stack_changed(project.root());
    Ok(ProjectRemoval::builder()
        .project(ProjectIdentityView::from(project))
        .removed(removed)
        .stopped(stopped)
        .build())
}

/// The human report of `removal`: what became of the processes, then the
/// project forgotten.
pub fn removal_rows(removal: &ProjectRemoval) -> Report {
    let mut rows: Vec<Row> = action_rows(removal.stopped()).rows().clone();
    rows.push(Row::unlabeled(
        RowKind::Ok,
        format!(
            "{REMOVED} {}{COLUMN_GAP}{}",
            removal.project().name(),
            removal.project().root().display()
        ),
    ));
    Report::new(PROJECTS_TITLE, rows).with_failure(removal.stopped().failed())
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
            .processes(processes)
            .current(current)
            .build()
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
