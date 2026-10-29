//! `muster projects`: the registered projects, changing one's idle stop,
//! and removing one.

use std::{
    fs,
    path::{Path, PathBuf},
};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    idle_stop_override::IdleStopOverride,
    idle_stop_source::IdleStopSource,
    idle_stop_view::IdleStopView,
    output::{COLUMN_GAP, action_rows, table_lines},
    project_removal::ProjectRemoval,
    report::Report,
    row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::{
        client::{ProjectIdentityView, ServiceClient, SettleWatch, process::down},
        path::expand_home,
        sanitize::printable,
    },
    application::StackEditor,
    domain::{
        config::ConfigError,
        port::{LocalState, StateChangeNotifier},
        project::Project,
        value::{IdleStop, ProjectName, ProjectRoot},
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
/// Names a project's idle stop in its rows.
const IDLE_STOP_LABEL: &str = "idle-stop";
/// Says a project's idle stop is the setting's, before the setting's value.
const FOLLOWS_SETTING: &str = "follows the setting";
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
    /// The idle stop in effect, its own or the setting's: whole seconds in
    /// JSON, or null for never or when its stored row does not read.
    #[serde(serialize_with = "crate::adapter::client::idle_stop_seconds::serialize_optional")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    idle_stop: Option<IdleStop>,
    /// Where its idle stop comes from, or none when its stored row does not
    /// read.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    source: Option<IdleStopSource>,
}

/// Every registered project, the one containing `cwd` marked current, then
/// every project whose stored row does not read, by folder. A project whose
/// row or processes do not read is listed with the reason, so one corrupt
/// row never hides the others. Each readable project carries the idle stop
/// in effect and its source.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn list(store: &dyn LocalState, cwd: &Path) -> Result<Vec<ProjectListing>, CliError> {
    let current = match store.project_containing(cwd) {
        Ok(current) => current,
        Err(ConfigError::CorruptRow { .. }) => None,
        Err(error) => return Err(error.into()),
    };
    let setting = *store.load()?.idle_stop();
    let readable = store.projects()?.into_iter().map(|project| {
        let (processes, invalid) = match store.stack(project.root()) {
            Ok(stack) => (Some(stack.map_or(0, |stack| stack.processes().len())), None),
            Err(error @ ConfigError::CorruptRow { .. }) => (None, Some(error.to_string())),
            Err(error) => return Err(error.into()),
        };
        let (idle_stop, source) = match project.idle_stop() {
            Some(own) => (*own, IdleStopSource::Project),
            None => (setting, IdleStopSource::Setting),
        };
        Ok(ProjectListing::builder()
            .current(current.as_ref() == Some(&project))
            .name(Some(project.name().clone()))
            .root(project.root().as_ref().clone())
            .processes(processes)
            .invalid(invalid)
            .idle_stop(Some(idle_stop))
            .source(Some(source))
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

/// The human list of `listings`: `name  folder  N processes`, then the
/// project's own idle stop when it has one, aligned, the current project
/// marked.
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
                match (listing.source(), listing.idle_stop()) {
                    (Some(IdleStopSource::Project), Some(own)) => {
                        format!("{IDLE_STOP_LABEL} {own}")
                    },
                    _ => String::new(),
                },
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

/// Gives `project` the idle stop `choice` names through `editor`, which
/// tells a running service, and returns the idle stop now in effect, read
/// from the setting in `store` when the project follows it.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read or written.
pub fn set_idle_stop(
    store: &dyn LocalState,
    editor: &StackEditor<'_>,
    project: &Project,
    choice: IdleStopOverride,
) -> Result<IdleStopView, CliError> {
    let updated = editor.set_idle_stop(project.root(), choice.own())?;
    let (idle_stop, source) = match updated.idle_stop() {
        Some(own) => (*own, IdleStopSource::Project),
        None => (*store.load()?.idle_stop(), IdleStopSource::Setting),
    };
    Ok(IdleStopView::builder()
        .project(ProjectIdentityView::from(&updated))
        .idle_stop(idle_stop)
        .source(source)
        .build())
}

/// The framed line `projects set` prints: `shop idle-stop 10m`, or `shop
/// idle-stop follows the setting (30m)`.
pub fn idle_stop_rows(view: &IdleStopView) -> Report {
    let value = match view.source() {
        IdleStopSource::Project => view.idle_stop().to_string(),
        IdleStopSource::Setting => format!("{FOLLOWS_SETTING} ({})", view.idle_stop()),
    };
    Report::new(PROJECTS_TITLE, vec![Row::unlabeled(
        RowKind::Ok,
        format!("{} {IDLE_STOP_LABEL} {value}", view.project().name()),
    )])
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

/// The stored root of the project whose row does not read that `named`
/// names, either as the exact text it is listed under or as a folder given
/// against `cwd`; none when no such project is there.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be read.
pub fn unreadable_named(
    store: &dyn LocalState,
    named: &str,
    cwd: &Path,
) -> Result<Option<PathBuf>, CliError> {
    let typed = Path::new(named);
    let path = cwd.join(expand_home(typed));
    let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    Ok(store
        .unreadable_projects()?
        .into_iter()
        .map(|project| project.root().clone())
        .find(|stored| stored == typed || stored == &path || stored == &resolved))
}

/// Forgets the project stored at `stored`, whose row does not read, with its
/// processes, environment and coordination state, and tells the service when
/// `stored` is a folder it could know. No service could load it, so nothing
/// of it runs to stop first.
///
/// # Errors
/// Returns the [`CliError`] of a store that cannot be written.
pub fn remove_unreadable(
    store: &dyn LocalState,
    notifier: &dyn StateChangeNotifier,
    stored: &Path,
) -> Result<ProjectRemoval, CliError> {
    let removed = store.remove_unreadable_project(stored)?;
    if let Ok(root) = ProjectRoot::try_new(stored.to_path_buf()) {
        notifier.stack_changed(&root);
    }
    Ok(ProjectRemoval::builder()
        .name(None)
        .root(stored.to_path_buf())
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
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::report::plain,
            client::recording_notifier::RecordingNotifier,
            store::{
                SqliteStore,
                test_support::{fresh_store, spec},
            },
        },
        domain::{
            config::ImportMode,
            port::{ProjectStore, SettingsReader, StackStore},
        },
    };

    /// A listing of `name` at `root` with `processes`, `current` or not,
    /// following a 30 minute idle-stop setting.
    fn listing(name: &str, root: &str, processes: usize, current: bool) -> ProjectListing {
        ProjectListing::builder()
            .name(Some(ProjectName::try_new(name).unwrap()))
            .root(PathBuf::from(root))
            .processes(Some(processes))
            .current(current)
            .idle_stop(Some(idle("30m")))
            .source(Some(IdleStopSource::Setting))
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
            serde_json::json!({"name": null, "root": "/srv/odd", "processes": null, "current": false, "invalid": "its name is refused", "idle_stop": null, "source": null})
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
            serde_json::json!({"name": "blog", "root": "/srv/blog", "processes": null, "current": false, "invalid": "stored stop grace is invalid", "idle_stop": null, "source": null})
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

    /// The JSON names, roots, counts and marks each project, with the idle
    /// stop in effect in whole seconds, null for never, and its source.
    #[test]
    fn a_listing_reads_as_json() {
        assert_eq!(
            serde_json::to_value(listing("shop", "/home/me/shop", 3, true)).unwrap(),
            serde_json::json!({"name": "shop", "root": "/home/me/shop", "processes": 3, "current": true, "idle_stop": 1800, "source": "setting"})
        );
        let never = ProjectListing::builder()
            .name(Some(ProjectName::try_new("blog").unwrap()))
            .root(PathBuf::from("/srv/blog"))
            .processes(Some(1))
            .current(false)
            .idle_stop(Some(IdleStop::Never))
            .source(Some(IdleStopSource::Project))
            .build();
        assert_eq!(
            serde_json::to_value(never).unwrap(),
            serde_json::json!({"name": "blog", "root": "/srv/blog", "processes": 1, "current": false, "idle_stop": null, "source": "project"})
        );
    }

    /// A store holding the project `shop` with one process, in a scratch
    /// directory, and the project as stored.
    fn shop_store() -> (SqliteStore, Project, TempDir) {
        let (store, scratch) = fresh_store();
        let root = ProjectRoot::try_new(scratch.path().join("shop")).unwrap();
        store
            .import(&root, &[spec("api", "true")], ImportMode::Strict)
            .unwrap();
        let project = store.project_at(&root).unwrap().unwrap();
        (store, project, scratch)
    }

    /// `idle_stop` as written.
    fn idle(raw: &str) -> IdleStop {
        raw.parse().unwrap()
    }

    /// A duration and `never` are stored as the project's own, `default`
    /// clears it, each change is told to the service, and each answer names
    /// the idle stop in effect and where it comes from.
    #[test]
    fn projects_set_stores_a_duration_never_and_default() {
        let (store, project, _scratch) = shop_store();
        let notifier = RecordingNotifier::default();
        let editor = StackEditor::builder()
            .stacks(&store)
            .settings(&store)
            .notifier(&notifier)
            .build();
        let stored = || {
            *store
                .project_at(project.root())
                .unwrap()
                .unwrap()
                .idle_stop()
        };

        let ten = set_idle_stop(
            &store,
            &editor,
            &project,
            IdleStopOverride::Own(idle("10m")),
        )
        .unwrap();
        assert_eq!(stored(), Some(idle("10m")));
        assert_eq!(
            (ten.idle_stop(), ten.source()),
            (idle("10m"), IdleStopSource::Project)
        );

        let never = set_idle_stop(
            &store,
            &editor,
            &project,
            IdleStopOverride::Own(IdleStop::Never),
        )
        .unwrap();
        assert_eq!(stored(), Some(IdleStop::Never));
        assert_eq!(never.idle_stop(), IdleStop::Never);

        let inherited =
            set_idle_stop(&store, &editor, &project, IdleStopOverride::Setting).unwrap();
        assert_eq!(stored(), None);
        assert_eq!(
            (inherited.idle_stop(), inherited.source()),
            (*store.load().unwrap().idle_stop(), IdleStopSource::Setting)
        );
        assert_eq!(notifier.stacks().lock().unwrap().len(), 3);
        assert_eq!(plain(&idle_stop_rows(&ten)), "shop idle-stop 10m");
        assert_eq!(
            plain(&idle_stop_rows(&inherited)),
            "shop idle-stop follows the setting (30m)"
        );
    }

    /// The JSON of `projects set` names the project, the idle stop in
    /// effect, and its source.
    #[test]
    fn projects_set_json_names_the_source() {
        let (store, project, _scratch) = shop_store();
        let notifier = RecordingNotifier::default();
        let editor = StackEditor::builder()
            .stacks(&store)
            .settings(&store)
            .notifier(&notifier)
            .build();
        let views: Vec<serde_json::Value> = [
            IdleStopOverride::Own(idle("10m")),
            IdleStopOverride::Own(IdleStop::Never),
            IdleStopOverride::Setting,
        ]
        .into_iter()
        .map(|choice| {
            let mut json =
                serde_json::to_value(set_idle_stop(&store, &editor, &project, choice).unwrap())
                    .unwrap();
            json["project"]["root"] = serde_json::json!("/home/me/shop");
            json
        })
        .collect();

        insta::assert_snapshot!(serde_json::to_string_pretty(&views).unwrap());
    }

    /// `muster projects` lists a project's own idle stop, and nothing for one
    /// following the setting; its JSON gives each the idle stop in effect
    /// and where it comes from.
    #[test]
    fn projects_lists_the_override() {
        let (store, project, scratch) = shop_store();
        let blog = ProjectRoot::try_new(scratch.path().join("blog")).unwrap();
        store
            .import(&blog, &[spec("web", "true")], ImportMode::Strict)
            .unwrap();
        store
            .set_idle_stop(project.root(), Some(idle("10m")))
            .unwrap();

        let listings = list(&store, scratch.path()).unwrap();

        let in_effect: Vec<(Option<IdleStop>, Option<IdleStopSource>)> = listings
            .iter()
            .map(|listing| (listing.idle_stop(), listing.source()))
            .collect();
        assert_eq!(in_effect, vec![
            (
                Some(*store.load().unwrap().idle_stop()),
                Some(IdleStopSource::Setting)
            ),
            (Some(idle("10m")), Some(IdleStopSource::Project))
        ]);
        let rows = plain(&list_rows(&listings));
        assert!(rows.ends_with("1 process  idle-stop 10m"), "{rows}");
        assert!(
            !rows.lines().next().unwrap().contains("idle-stop"),
            "{rows}"
        );
        let json: Vec<serde_json::Value> = listings
            .iter()
            .map(|listing| {
                let json = serde_json::to_value(listing).unwrap();
                serde_json::json!([json["idle_stop"], json["source"]])
            })
            .collect();
        assert_eq!(json, [
            serde_json::json!([1800, "setting"]),
            serde_json::json!([600, "project"])
        ]);
    }
}
