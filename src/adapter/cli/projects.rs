//! `muster projects`: the registered projects, and removing one.

use std::{
    fs,
    path::{Path, PathBuf},
};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    output::{COLUMN_GAP, action_rows, table_lines},
    project_removal::ProjectRemoval,
    report::Report,
    row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::{
        client::{ServiceClient, SettleWatch, process::down},
        path::expand_home,
        sanitize::printable,
    },
    domain::{
        config::ConfigError,
        port::{LocalState, StateChangeNotifier},
        project::Project,
        value::{ProjectName, ProjectRoot},
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
/// Stands for the name of a project whose stored row does not read.
const UNNAMED: &str = "?";
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
    /// The project's name, or none when its stored row does not read.
    #[getset(get = "pub")]
    name: Option<ProjectName>,
    /// Its folder.
    #[getset(get = "pub")]
    root: PathBuf,
    /// How many processes it has, or none when its stored processes do not
    /// read.
    #[getset(get_copy = "pub")]
    processes: Option<usize>,
    /// Why its stored row or processes do not read, when they do not.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    invalid: Option<String>,
    /// Whether it contains the current folder.
    #[getset(get_copy = "pub")]
    current: bool,
}

/// Every registered project, the one containing `cwd` marked current, then
/// every project whose stored row does not read, by folder. A project whose
/// row or processes do not read is listed with the reason, so one corrupt
/// row never hides the others.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn list(store: &dyn LocalState, cwd: &Path) -> Result<Vec<ProjectListing>, CliError> {
    let current = match store.project_containing(cwd) {
        Ok(current) => current,
        Err(ConfigError::CorruptRow { .. }) => None,
        Err(error) => return Err(error.into()),
    };
    let readable = store.projects()?.into_iter().map(|project| {
        let (processes, invalid) = match store.stack(project.root()) {
            Ok(stack) => (Some(stack.map_or(0, |stack| stack.processes().len())), None),
            Err(error @ ConfigError::CorruptRow { .. }) => (None, Some(error.to_string())),
            Err(error) => return Err(error.into()),
        };
        Ok(ProjectListing::builder()
            .current(current.as_ref() == Some(&project))
            .name(Some(project.name().clone()))
            .root(project.root().as_ref().clone())
            .processes(processes)
            .invalid(invalid)
            .build())
    });
    let unreadable = store.unreadable_projects()?.into_iter().map(|project| {
        Ok(ProjectListing::builder()
            .name(None)
            .root(project.root().clone())
            .processes(None)
            .invalid(Some(project.reason().clone()))
            .current(false)
            .build())
    });
    readable.chain(unreadable).collect()
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
                listing
                    .name()
                    .as_ref()
                    .map_or_else(|| UNNAMED.to_string(), ToString::to_string),
                printable(&listing.root().to_string_lossy()),
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
        .name(Some(project.name().clone()))
        .root(project.root().as_ref().clone())
        .removed(removed)
        .stopped(stopped)
        .build())
}

/// The folder of the project whose stored row does not read that `named`,
/// a folder given against `cwd`, names; none when no such project is there.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn unreadable_named(
    store: &dyn LocalState,
    named: &str,
    cwd: &Path,
) -> Result<Option<ProjectRoot>, CliError> {
    let path = cwd.join(expand_home(Path::new(named)));
    let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    Ok(store
        .unreadable_projects()?
        .into_iter()
        .find(|project| project.root() == &path || project.root() == &resolved)
        .and_then(|project| ProjectRoot::try_new(project.root().clone()).ok()))
}

/// Forgets the project at `root`, whose stored row does not read, with its
/// processes, environment and coordination state, and tells the service. No
/// service could load it, so nothing of it runs to stop first.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be written.
pub fn remove_unreadable(
    store: &dyn LocalState,
    notifier: &dyn StateChangeNotifier,
    root: &ProjectRoot,
) -> Result<ProjectRemoval, CliError> {
    let removed = store.remove_project(root)?;
    notifier.stack_changed(root);
    Ok(ProjectRemoval::builder()
        .name(None)
        .root(root.as_ref().clone())
        .removed(removed)
        .stopped(None)
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
            removal
                .name()
                .as_ref()
                .map_or_else(|| UNNAMED.to_string(), ToString::to_string),
            printable(&removal.root().to_string_lossy())
        ),
    ));
    Report::new(PROJECTS_TITLE, rows).with_failure(removal.failed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::cli::report::plain;

    /// A listing of `name` at `root` with `processes`, `current` or not.
    fn listing(name: &str, root: &str, processes: usize, current: bool) -> ProjectListing {
        ProjectListing::builder()
            .name(Some(ProjectName::try_new(name).unwrap()))
            .root(PathBuf::from(root))
            .processes(Some(processes))
            .current(current)
            .build()
    }

    /// A project whose stored row does not read is listed by its folder,
    /// with the reason and no name.
    #[test]
    fn a_project_whose_row_does_not_read_is_listed_by_its_folder() {
        let broken = ProjectListing::builder()
            .name(None)
            .root(PathBuf::from("/srv/odd"))
            .processes(None)
            .invalid(Some("its name is refused".to_string()))
            .current(false)
            .build();

        assert_eq!(
            plain(&list_rows(&[
                listing("shop", "/home/me/shop", 3, true),
                broken.clone()
            ])),
            "* shop  /home/me/shop  3 processes\n  ?     /srv/odd       unreadable: its name is \
             refused"
        );
        assert_eq!(
            serde_json::to_value(broken).unwrap(),
            serde_json::json!({"name": null, "root": "/srv/odd", "processes": null, "current": false, "invalid": "its name is refused"})
        );
    }

    /// A project whose processes do not read is listed with the reason.
    #[test]
    fn an_unreadable_project_is_listed_with_its_reason() {
        let broken = ProjectListing::builder()
            .name(Some(ProjectName::try_new("blog").unwrap()))
            .root(PathBuf::from("/srv/blog"))
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
