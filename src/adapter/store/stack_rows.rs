use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, Row, ffi, params, params_from_iter, types::Value};

use super::{
    project_rows::{ProjectRow, find_project, register, require_project},
    sqlite_store::{
        SqliteStore, corrupt, db_error, duration_millis, grace_from_millis, positive_from_millis,
    },
};
use crate::domain::{
    config::{
        ConfigError, Environment, ImportMode, ImportPlan, ProcessChange, ProcessSpec, Registration,
        Stack,
    },
    port::StackStore,
    process::{ReadyCheck, ReadyProbe, StopPolicy},
    project::Project,
    value::{CommandLine, Description, EnvName, EnvValue, ProcessName, ProjectRoot},
};

/// Selects a process's definition columns in the order
/// [`process_from_row`] reads them.
const SELECT_PROCESS: &str = "SELECT id, name, command, working_dir, description, autostart, \
                              restart, stop_signal, stop_grace_ms, ready_kind, ready_target, \
                              ready_timeout_ms FROM processes";
/// A process's stop grace, as its errors name it.
const STOP_GRACE: &str = "stop grace";
/// A process's readiness timeout, as its errors name it.
const READY_TIMEOUT: &str = "readiness timeout";

/// The text a path column holds.
///
/// # Errors
/// Returns [`ConfigError::Database`] for a path that is not UTF-8.
fn path_text(path: &Path) -> Result<&str, ConfigError> {
    path.to_str()
        .ok_or_else(|| ConfigError::Database(format!("{} is not valid UTF-8", path.display())))
}

/// Reads a [`SELECT_PROCESS`] row, taking its variables from `environments`.
fn process_from_row(
    row: &Row,
    environments: &mut HashMap<i64, Environment>,
) -> Result<ProcessSpec, ConfigError> {
    let id: i64 = row.get(0).map_err(db_error)?;
    let name: String = row.get(1).map_err(db_error)?;
    let command: String = row.get(2).map_err(db_error)?;
    let working_dir: Option<String> = row.get(3).map_err(db_error)?;
    let description: Option<String> = row.get(4).map_err(db_error)?;
    let autostart: bool = row.get(5).map_err(db_error)?;
    let restart: String = row.get(6).map_err(db_error)?;
    let stop_signal: String = row.get(7).map_err(db_error)?;
    let stop_grace_ms: i64 = row.get(8).map_err(db_error)?;
    let ready_kind: Option<String> = row.get(9).map_err(db_error)?;
    let ready_target: Option<String> = row.get(10).map_err(db_error)?;
    let ready_timeout_ms: Option<i64> = row.get(11).map_err(db_error)?;
    let ready = match (ready_kind, ready_target, ready_timeout_ms) {
        (Some(kind), Some(target), Some(timeout_ms)) => Some(
            ReadyCheck::builder()
                .probe(
                    ReadyProbe::from_stored(&kind, &target)
                        .map_err(|error| corrupt("readiness check", error))?,
                )
                .timeout(positive_from_millis(READY_TIMEOUT, timeout_ms)?)
                .build(),
        ),
        _ => None,
    };
    Ok(ProcessSpec::builder()
        .name(ProcessName::try_new(name).map_err(|error| corrupt("process name", error))?)
        .command(CommandLine::try_new(command).map_err(|error| corrupt("command", error))?)
        .working_dir(working_dir.map(PathBuf::from))
        .description(
            description
                .map(Description::try_new)
                .transpose()
                .map_err(|error| corrupt("description", error))?,
        )
        .environment(environments.remove(&id).unwrap_or_else(Environment::empty))
        .autostart(autostart)
        .restart(
            restart
                .parse()
                .map_err(|error| corrupt("restart policy", error))?,
        )
        .stop(
            StopPolicy::builder()
                .signal(
                    stop_signal
                        .parse()
                        .map_err(|error| corrupt("stop signal", error))?,
                )
                .grace_period(grace_from_millis(STOP_GRACE, stop_grace_ms)?)
                .build(),
        )
        .ready(ready)
        .build())
}

/// Every variable of the project's processes, by process row id, read in one
/// query.
fn environments_of(
    conn: &Connection,
    project_id: i64,
) -> Result<HashMap<i64, Environment>, ConfigError> {
    let mut statement = conn
        .prepare(
            "SELECT e.process_id, e.name, e.value FROM process_env e \
             JOIN processes p ON p.id = e.process_id WHERE p.project_id = ?1",
        )
        .map_err(db_error)?;
    let mut rows = statement.query(params![project_id]).map_err(db_error)?;
    let mut environments: HashMap<i64, Environment> = HashMap::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        let process_id: i64 = row.get(0).map_err(db_error)?;
        let name: String = row.get(1).map_err(db_error)?;
        let value: String = row.get(2).map_err(db_error)?;
        let name = EnvName::try_new(name).map_err(|error| corrupt("variable name", error))?;
        let value = EnvValue::try_new(value).map_err(|error| corrupt("variable value", error))?;
        let environment = environments
            .remove(&process_id)
            .unwrap_or_else(Environment::empty)
            .with_var(name, value);
        environments.insert(process_id, environment);
    }
    Ok(environments)
}

/// The stack of the project `(project_id, project)`, in insertion order.
fn stack_of(conn: &Connection, (project_id, project): ProjectRow) -> Result<Stack, ConfigError> {
    let mut environments = environments_of(conn, project_id)?;
    let mut statement = conn
        .prepare(&format!(
            "{SELECT_PROCESS} WHERE project_id = ?1 ORDER BY id"
        ))
        .map_err(db_error)?;
    let mut rows = statement.query(params![project_id]).map_err(db_error)?;
    let mut processes = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        processes.push(process_from_row(row, &mut environments)?);
    }
    Ok(Stack::builder()
        .project(project)
        .processes(processes)
        .build())
}

/// Writes `spec`'s variables for the process row `process_id`.
fn insert_environment(
    conn: &Connection,
    process_id: i64,
    spec: &ProcessSpec,
) -> Result<(), ConfigError> {
    let mut statement = conn
        .prepare("INSERT INTO process_env (process_id, name, value) VALUES (?1, ?2, ?3)")
        .map_err(db_error)?;
    for (name, value) in spec.environment().vars() {
        statement
            .execute(params![process_id, name.as_ref(), value.as_ref()])
            .map_err(db_error)?;
    }
    Ok(())
}

/// Whether `error` is a `UNIQUE` constraint failure.
fn is_unique_violation(error: &rusqlite::Error) -> bool {
    error.sqlite_extended_error_code() == Some(ffi::SQLITE_CONSTRAINT_UNIQUE)
}

/// What the insert and update statements bind: the project id as `?1`, the
/// name key as `?2`, then the definition columns `?3` to `?13`.
///
/// # Errors
/// Returns [`ConfigError::Database`] for a working directory that is not UTF-8.
fn process_params(project_id: i64, spec: &ProcessSpec) -> Result<Vec<Value>, ConfigError> {
    let working_dir = spec
        .working_dir()
        .as_deref()
        .map(path_text)
        .transpose()?
        .map(str::to_owned);
    let ready = spec
        .ready()
        .as_ref()
        .map(|check| {
            let (kind, target) = check.probe().stored();
            duration_millis(READY_TIMEOUT, check.timeout().into_inner())
                .map(|timeout| (kind.to_string(), target, timeout))
        })
        .transpose()?;
    let (ready_kind, ready_target, ready_timeout_ms) = match ready {
        Some((kind, target, timeout)) => (Some(kind), Some(target), Some(timeout)),
        None => (None, None, None),
    };
    Ok(vec![
        project_id.into(),
        spec.name().key().into(),
        spec.name().to_string().into(),
        spec.command().clone().into_inner().into(),
        working_dir.into(),
        spec.description()
            .clone()
            .map(Description::into_inner)
            .into(),
        (*spec.autostart()).into(),
        spec.restart().to_string().into(),
        spec.stop().signal().to_string().into(),
        duration_millis(STOP_GRACE, spec.stop().grace_period().into_inner())?.into(),
        ready_kind.into(),
        ready_target.into(),
        ready_timeout_ms.into(),
    ])
}

/// Inserts `spec` into the project `project_id`.
///
/// # Errors
/// Returns [`ConfigError::DuplicateProcessName`] when its name's key is taken.
fn insert_process(
    conn: &Connection,
    project_id: i64,
    spec: &ProcessSpec,
) -> Result<(), ConfigError> {
    let process_id: i64 = conn
        .query_row(
            "INSERT INTO processes (project_id, name_key, name, command, working_dir, description, \
             autostart, restart, stop_signal, stop_grace_ms, ready_kind, ready_target, \
             ready_timeout_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) RETURNING id",
            params_from_iter(process_params(project_id, spec)?),
            |row| row.get(0),
        )
        .map_err(|error| {
            if is_unique_violation(&error) {
                ConfigError::DuplicateProcessName {
                    name: spec.name().clone(),
                }
            } else {
                db_error(error)
            }
        })?;
    insert_environment(conn, process_id, spec)
}

/// Overwrites the definition of the process whose name shares `spec`'s key,
/// keeping its row id, so its place in the stack stays.
fn update_process(
    conn: &Connection,
    project_id: i64,
    spec: &ProcessSpec,
) -> Result<(), ConfigError> {
    let process_id: i64 = conn
        .query_row(
            "UPDATE processes SET name = ?3, command = ?4, working_dir = ?5, description = ?6, \
             autostart = ?7, restart = ?8, stop_signal = ?9, stop_grace_ms = ?10, \
             ready_kind = ?11, ready_target = ?12, ready_timeout_ms = ?13 \
             WHERE project_id = ?1 AND name_key = ?2 RETURNING id",
            params_from_iter(process_params(project_id, spec)?),
            |row| row.get(0),
        )
        .map_err(db_error)?;
    conn.execute("DELETE FROM process_env WHERE process_id = ?1", params![
        process_id
    ])
    .map_err(db_error)?;
    insert_environment(conn, process_id, spec)
}

/// Deletes the process whose name shares `name`'s key; its variables cascade.
fn delete_process(
    conn: &Connection,
    project_id: i64,
    name: &ProcessName,
) -> Result<(), ConfigError> {
    conn.execute(
        "DELETE FROM processes WHERE project_id = ?1 AND name_key = ?2",
        params![project_id, name.key()],
    )
    .map_err(db_error)?;
    Ok(())
}

/// The spec in `processes` whose name shares `name`'s key.
fn named<'a>(processes: &'a [ProcessSpec], name: &ProcessName) -> Option<&'a ProcessSpec> {
    let key = name.key();
    processes.iter().find(|spec| spec.name().key() == key)
}

impl StackStore for SqliteStore {
    fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
        self.read(|conn| {
            find_project(conn, root)?
                .map(|project| stack_of(conn, project))
                .transpose()
        })
    }

    fn add_process(
        &self,
        root: &ProjectRoot,
        spec: &ProcessSpec,
    ) -> Result<Registration, ConfigError> {
        self.write(|conn| {
            let (project_id, registration) = register(conn, root)?;
            insert_process(conn, project_id, spec)?;
            Ok(registration)
        })
    }

    fn change_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
    ) -> Result<ProcessSpec, ConfigError> {
        self.write(|conn| {
            let project = require_project(conn, root)?;
            let project_id = project.0;
            let changed = stack_of(conn, project)?
                .require(name)?
                .clone()
                .with_change(change);
            update_process(conn, project_id, &changed)?;
            Ok(changed)
        })
    }

    fn remove_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
    ) -> Result<ProcessSpec, ConfigError> {
        self.write(|conn| {
            let project = require_project(conn, root)?;
            let project_id = project.0;
            let removed = stack_of(conn, project)?.require(name)?.clone();
            delete_process(conn, project_id, name)?;
            Ok(removed)
        })
    }

    fn import(
        &self,
        root: &ProjectRoot,
        processes: &[ProcessSpec],
        mode: ImportMode,
    ) -> Result<(Registration, ImportPlan), ConfigError> {
        self.write(|conn| {
            let (project_id, registration) = register(conn, root)?;
            let project: Project = registration.project().clone();
            let plan = stack_of(conn, (project_id, project))?.plan_import(processes, mode)?;
            for name in plan.removed() {
                delete_process(conn, project_id, name)?;
            }
            for name in plan.updated() {
                if let Some(spec) = named(processes, name) {
                    update_process(conn, project_id, spec)?;
                }
            }
            for name in plan.added() {
                if let Some(spec) = named(processes, name) {
                    insert_process(conn, project_id, spec)?;
                }
            }
            Ok((registration, plan))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{fresh_store, spec},
        domain::{
            config::FieldEdit,
            port::ProjectStore,
            process::{RestartPolicy, StopSignal},
            value::PositiveDuration,
        },
    };

    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    fn root(scratch: &TempDir) -> ProjectRoot {
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap()
    }

    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    fn full_spec() -> ProcessSpec {
        spec("web", "npm run dev")
            .with_working_dir(Some(PathBuf::from("frontend")))
            .with_description(Some(Description::try_new("Vite dev server").unwrap()))
            .with_environment(
                Environment::empty()
                    .with_var(
                        EnvName::try_new("PORT").unwrap(),
                        EnvValue::try_new("5173").unwrap(),
                    )
                    .with_var(
                        EnvName::try_new("EMPTY").unwrap(),
                        EnvValue::try_new("").unwrap(),
                    ),
            )
            .with_autostart(true)
            .with_restart(RestartPolicy::OnFailure)
    }

    fn names_of(stack: &Stack) -> Vec<String> {
        stack.names().iter().map(ToString::to_string).collect()
    }

    /// A readiness check on local port 3000.
    fn port_check() -> ReadyCheck {
        ReadyCheck::builder()
            .probe(ReadyProbe::from_stored("port", "3000").unwrap())
            .timeout(positive_from_millis(READY_TIMEOUT, 30_000).unwrap())
            .build()
    }

    fn ready_target(store: &SqliteStore, process: &str) -> Option<String> {
        store
            .read(|conn| {
                conn.query_row(
                    "SELECT ready_target FROM processes WHERE name = ?1",
                    params![process],
                    |row| row.get(0),
                )
                .map_err(db_error)
            })
            .unwrap()
    }

    #[test]
    fn a_definition_round_trips_in_insertion_order() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        assert_eq!(store.stack(&root).unwrap(), None, "not registered yet");

        store.add_process(&root, &full_spec()).unwrap();
        store.add_process(&root, &spec("api", "cargo run")).unwrap();
        store.add_process(&root, &spec("db", "postgres")).unwrap();

        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(names_of(&stack), ["web", "api", "db"]);
        assert_eq!(stack.processes()[0], full_spec());
        assert_eq!(stack.project().root(), &root);
    }

    #[test]
    fn a_name_taken_by_nfc_key_is_refused() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &spec(CAFE_COMPOSED, "true"))
            .unwrap();

        let error = store
            .add_process(&root, &spec(CAFE_DECOMPOSED, "false"))
            .unwrap_err();

        assert!(matches!(
            error,
            ConfigError::DuplicateProcessName { name } if name.as_ref() == CAFE_DECOMPOSED
        ));
        assert_eq!(store.stack(&root).unwrap().unwrap().processes().len(), 1);
    }

    /// A change rewrites the definition in place: its order, its variables
    /// and its readiness check stay where the change does not touch them.
    #[test]
    fn a_change_keeps_order_and_readiness() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &full_spec().with_ready(Some(port_check())))
            .unwrap();
        store.add_process(&root, &spec("api", "cargo run")).unwrap();
        let change = ProcessChange::builder()
            .command(Some(CommandLine::try_new("pnpm dev").unwrap()))
            .working_dir(FieldEdit::Clear)
            .unset_env(vec![EnvName::try_new("EMPTY").unwrap()])
            .stop_signal(Some(StopSignal::Interrupt))
            .build();

        let changed = store.change_process(&root, &name("web"), &change).unwrap();

        assert_eq!(
            changed,
            full_spec()
                .with_ready(Some(port_check()))
                .with_change(&change)
        );
        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(names_of(&stack), ["web", "api"]);
        assert_eq!(stack.processes()[0], changed);
        assert_eq!(ready_target(&store, "web").as_deref(), Some("3000"));
    }

    /// Each readiness probe is written and read back, and a change sets,
    /// replaces and clears it.
    #[test]
    fn a_readiness_check_is_stored_and_changed() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let check = |kind: &str, target: &str| {
            ReadyCheck::builder()
                .probe(ReadyProbe::from_stored(kind, target).unwrap())
                .timeout(positive_from_millis(READY_TIMEOUT, 60_000).unwrap())
                .build()
        };
        let regex = check("regex", r"ready in \d+ms");
        store
            .add_process(
                &root,
                &spec("web", "npm run dev").with_ready(Some(regex.clone())),
            )
            .unwrap();
        assert_eq!(
            store.stack(&root).unwrap().unwrap().processes()[0].ready(),
            &Some(regex)
        );

        let http = check("http", "http://localhost:3000/health");
        let set = ProcessChange::builder()
            .ready(FieldEdit::Set(http.clone()))
            .build();
        assert_eq!(
            store
                .change_process(&root, &name("web"), &set)
                .unwrap()
                .ready(),
            &Some(http)
        );

        let clear = ProcessChange::builder().ready(FieldEdit::Clear).build();
        store.change_process(&root, &name("web"), &clear).unwrap();
        assert_eq!(
            store.stack(&root).unwrap().unwrap().processes()[0].ready(),
            &None
        );
        assert_eq!(ready_target(&store, "web"), None);
    }

    /// A readiness timeout too long for a millisecond column is refused as
    /// the caller's input rather than wrapped, and writes nothing.
    #[test]
    fn a_readiness_timeout_too_large_to_store_is_refused() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let endless = ReadyCheck::builder()
            .probe(ReadyProbe::from_stored("port", "3000").unwrap())
            .timeout(PositiveDuration::try_new(Duration::MAX).unwrap())
            .build();

        let error = store
            .add_process(&root, &spec("web", "npm run dev").with_ready(Some(endless)))
            .unwrap_err();

        assert!(matches!(error, ConfigError::TooLarge { .. }), "{error}");
        assert!(store.stack(&root).unwrap().is_none());
    }

    #[test]
    fn changing_or_removing_an_unknown_process_names_the_valid_ones() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let change = ProcessChange::builder().autostart(Some(true)).build();
        assert!(matches!(
            store.change_process(&root, &name("web"), &change),
            Err(ConfigError::UnknownProject { root: refused }) if &refused == root.as_ref()
        ));
        store.add_process(&root, &spec("api", "cargo run")).unwrap();

        for error in [
            store
                .change_process(&root, &name("web"), &change)
                .unwrap_err(),
            store.remove_process(&root, &name("web")).unwrap_err(),
        ] {
            assert!(matches!(
                error,
                ConfigError::UnknownProcess { valid, .. } if valid == vec![name("api")]
            ));
        }
    }

    #[test]
    fn remove_returns_the_definition_and_keeps_the_project() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store.add_process(&root, &full_spec()).unwrap();

        let removed = store.remove_process(&root, &name("web")).unwrap();

        assert_eq!(removed, full_spec());
        assert!(store.stack(&root).unwrap().unwrap().processes().is_empty());
        assert!(store.project_at(&root).unwrap().is_some());
    }

    /// A refused strict import writes nothing, not even the registration.
    #[test]
    fn a_strict_conflict_writes_nothing() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);

        let error = store
            .import(
                &root,
                &[spec("web", "a"), spec("web", "b")],
                ImportMode::Strict,
            )
            .unwrap_err();
        assert!(matches!(error, ConfigError::DuplicateProcessName { .. }));
        assert_eq!(store.project_at(&root).unwrap(), None);

        store
            .add_process(&root, &spec("web", "npm run dev"))
            .unwrap();
        let error = store
            .import(
                &root,
                &[spec("worker", "w"), spec("web", "x")],
                ImportMode::Strict,
            )
            .unwrap_err();
        assert!(
            matches!(error, ConfigError::ImportConflict { names } if names == vec![name("web")])
        );
        assert_eq!(names_of(&store.stack(&root).unwrap().unwrap()), ["web"]);
    }

    #[test]
    fn a_strict_import_registers_a_new_folder() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);

        let (registration, plan) = store
            .import(
                &root,
                &[full_spec(), spec("api", "cargo run")],
                ImportMode::Strict,
            )
            .unwrap();

        assert!(*registration.newly_registered());
        assert_eq!(*plan.added(), vec![name("web"), name("api")]);
        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(stack.processes(), &vec![
            full_spec(),
            spec("api", "cargo run")
        ]);
    }

    /// Merge replaces same-named definitions in place, readiness included,
    /// and keeps the rest.
    #[test]
    fn a_merge_import_updates_in_place() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(
                &root,
                &spec("web", "npm run dev").with_ready(Some(port_check())),
            )
            .unwrap();
        store.add_process(&root, &spec("api", "cargo run")).unwrap();

        let (registration, plan) = store
            .import(
                &root,
                &[full_spec(), spec("worker", "bin/worker")],
                ImportMode::Merge,
            )
            .unwrap();

        assert!(!*registration.newly_registered());
        assert_eq!(*plan.updated(), vec![name("web")]);
        assert_eq!(*plan.added(), vec![name("worker")]);
        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(names_of(&stack), ["web", "api", "worker"]);
        assert_eq!(stack.processes()[0], full_spec());
        assert_eq!(ready_target(&store, "web"), None);
    }

    #[test]
    fn a_replace_import_leaves_exactly_the_file() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &spec("web", "npm run dev"))
            .unwrap();
        store.add_process(&root, &spec("api", "cargo run")).unwrap();

        let (_, plan) = store
            .import(
                &root,
                &[
                    spec("api", "cargo run --release"),
                    spec("worker", "bin/worker"),
                ],
                ImportMode::Replace,
            )
            .unwrap();

        assert_eq!(*plan.removed(), vec![name("web")]);
        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(stack.processes(), &vec![
            spec("api", "cargo run --release"),
            spec("worker", "bin/worker")
        ]);
    }
}
