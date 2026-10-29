use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use rusqlite::{
    Connection, Error as SqliteError, OptionalExtension, Row, ffi, params, params_from_iter,
    types::Value,
};

use super::{
    edit_log_rows::record_edit,
    project_rows::{
        ProjectRow, find_project, register, require_project, root_text, store_idle_stop,
    },
    sqlite_store::{
        SqliteStore, corrupt, db_error, duration_millis, grace_from_millis, positive_from_millis,
        refused,
    },
    store_error::StoreError,
};
use crate::domain::{
    config::{
        AppliedChange, ConfigError, Environment, ImportMode, ImportPlan, ProcessChange,
        ProcessSpec, Registration, Stack, StackEdit,
    },
    coordination::Author,
    port::{StackReader, StackStore},
    process::{ReadyCheck, ReadyProbe, StopPolicy},
    project::Project,
    value::{
        CommandLine, Description, EnvName, EnvValue, IdleStop, LocalPort, ProcessName, ProjectRoot,
        StackRevision,
    },
};

/// Selects the definition columns [`process_from_row`] reads by name.
const SELECT_PROCESS: &str = "SELECT id, name, command, working_dir, description, autostart, \
                              restart, stop_signal, stop_grace_ms, ready_kind, ready_target, \
                              ready_timeout_ms FROM processes";
/// A process's stop grace, as its errors name it.
const STOP_GRACE: &str = "stop grace";
/// A process's readiness timeout, as its errors name it.
const READY_TIMEOUT: &str = "readiness timeout";
/// A port a process declares, as its errors name it.
const DECLARED_PORT: &str = "declared port";

/// The text a path column holds.
///
/// # Errors
/// Returns a storage failure for a path that is not UTF-8.
fn path_text(path: &Path) -> Result<&str, ConfigError> {
    path.to_str()
        .ok_or_else(|| StoreError::NotUtf8(path.to_path_buf()).into())
}

/// Reads a [`SELECT_PROCESS`] row, taking its variables from `environments`
/// and its ports from `ports`. A working directory that leaves the root, or a
/// name or variables past a cap, read as stored, so the process can be
/// changed, removed or exported; [`ProcessSpec::stored_problem`] keeps it
/// from starting.
fn process_from_row(
    row: &Row,
    environments: &mut HashMap<i64, Environment>,
    ports: &mut HashMap<i64, BTreeSet<LocalPort>>,
) -> Result<ProcessSpec, ConfigError> {
    let id: i64 = row.get("id").map_err(db_error)?;
    let name: String = row.get("name").map_err(db_error)?;
    let command: String = row.get("command").map_err(db_error)?;
    let working_dir: Option<String> = row.get("working_dir").map_err(db_error)?;
    let description: Option<String> = row.get("description").map_err(db_error)?;
    let autostart: bool = row.get("autostart").map_err(db_error)?;
    let restart: String = row.get("restart").map_err(db_error)?;
    let stop_signal: String = row.get("stop_signal").map_err(db_error)?;
    let stop_grace_ms: i64 = row.get("stop_grace_ms").map_err(db_error)?;
    let ready_kind: Option<String> = row.get("ready_kind").map_err(db_error)?;
    let ready_target: Option<String> = row.get("ready_target").map_err(db_error)?;
    let ready_timeout_ms: Option<i64> = row.get("ready_timeout_ms").map_err(db_error)?;
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
    let name = ProcessName::try_new(name).map_err(|error| refused("process name", &error))?;
    Ok(ProcessSpec::builder()
        .name(name)
        .command(CommandLine::try_new(command).map_err(|error| refused("command", &error))?)
        .working_dir(working_dir.map(PathBuf::from))
        .description(
            description
                .map(Description::try_new)
                .transpose()
                .map_err(|error| refused("description", &error))?,
        )
        .environment(environments.remove(&id).unwrap_or_else(Environment::empty))
        .autostart(autostart)
        .restart(
            restart
                .parse()
                .map_err(|error| refused("restart policy", &error))?,
        )
        .stop(
            StopPolicy::builder()
                .signal(
                    stop_signal
                        .parse()
                        .map_err(|error| refused("stop signal", &error))?,
                )
                .grace_period(grace_from_millis(STOP_GRACE, stop_grace_ms)?)
                .build(),
        )
        .ready(ready)
        .ports(ports.remove(&id).unwrap_or_default())
        .build())
}

/// Every declared port of the project's processes, by process row id, read in
/// one query.
fn ports_of(
    conn: &Connection,
    project_id: i64,
) -> Result<HashMap<i64, BTreeSet<LocalPort>>, ConfigError> {
    let mut statement = conn
        .prepare(
            "SELECT pp.process_id, pp.port FROM process_ports pp \
             JOIN processes p ON p.id = pp.process_id WHERE p.project_id = ?1",
        )
        .map_err(db_error)?;
    let mut rows = statement.query(params![project_id]).map_err(db_error)?;
    let mut ports: HashMap<i64, BTreeSet<LocalPort>> = HashMap::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        let process_id: i64 = row.get("process_id").map_err(db_error)?;
        let port: i64 = row.get("port").map_err(db_error)?;
        let port = u16::try_from(port)
            .map_err(|error| refused(DECLARED_PORT, &error))
            .and_then(|port| {
                LocalPort::try_new(port).map_err(|error| refused(DECLARED_PORT, &error))
            })?;
        ports.entry(process_id).or_default().insert(port);
    }
    Ok(ports)
}

/// Writes `spec`'s declared ports for the process row `process_id`.
fn insert_ports(conn: &Connection, process_id: i64, spec: &ProcessSpec) -> Result<(), ConfigError> {
    let mut statement = conn
        .prepare("INSERT INTO process_ports (process_id, port) VALUES (?1, ?2)")
        .map_err(db_error)?;
    for port in spec.ports() {
        statement
            .execute(params![process_id, port.into_inner()])
            .map_err(db_error)?;
    }
    Ok(())
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
        let process_id: i64 = row.get("process_id").map_err(db_error)?;
        let name: String = row.get("name").map_err(db_error)?;
        let value: String = row.get("value").map_err(db_error)?;
        let name = EnvName::try_new(name).map_err(|error| refused("variable name", &error))?;
        let value = EnvValue::try_new(value).map_err(|error| refused("variable value", &error))?;
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
    let mut ports = ports_of(conn, project_id)?;
    let mut statement = conn
        .prepare(&format!(
            "{SELECT_PROCESS} WHERE project_id = ?1 ORDER BY id"
        ))
        .map_err(db_error)?;
    let mut rows = statement.query(params![project_id]).map_err(db_error)?;
    let mut processes = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        processes.push(process_from_row(row, &mut environments, &mut ports)?);
    }
    Ok(Stack::builder()
        .project(project)
        .processes(processes)
        .build())
}

/// How many processes the project `project_id` holds, readable or not.
fn process_count(conn: &Connection, project_id: i64) -> Result<usize, ConfigError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processes WHERE project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    usize::try_from(count).map_err(|error| refused("process count", &error))
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
fn is_unique_violation(error: &SqliteError) -> bool {
    error.sqlite_extended_error_code() == Some(ffi::SQLITE_CONSTRAINT_UNIQUE)
}

/// What the insert and update statements bind: the project id as `?1`, the
/// name key as `?2`, then the definition columns `?3` to `?13`.
///
/// # Errors
/// Returns a storage failure for a working directory that is not UTF-8.
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
    insert_environment(conn, process_id, spec)?;
    insert_ports(conn, process_id, spec)
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
    conn.execute("DELETE FROM process_ports WHERE process_id = ?1", params![
        process_id
    ])
    .map_err(db_error)?;
    insert_environment(conn, process_id, spec)?;
    insert_ports(conn, process_id, spec)
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

/// Moves the stack of the project `project_id` to the store's next revision,
/// inside the caller's write transaction.
///
/// # Errors
/// Returns a storage failure when the revision cannot be written.
fn bump_revision(conn: &Connection, project_id: i64) -> Result<(), ConfigError> {
    conn.execute(
        "UPDATE stack_revision_counter SET value = value + 1 WHERE id = 1",
        [],
    )
    .map_err(db_error)?;
    conn.execute(
        "UPDATE projects SET stack_revision = \
         (SELECT value FROM stack_revision_counter WHERE id = 1) WHERE id = ?1",
        params![project_id],
    )
    .map_err(db_error)?;
    Ok(())
}

/// The spec in `processes` whose name shares `name`'s key.
fn named<'a>(processes: &'a [ProcessSpec], name: &ProcessName) -> Option<&'a ProcessSpec> {
    let key = name.key();
    processes.iter().find(|spec| spec.name().key() == key)
}

impl StackReader for SqliteStore {
    fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
        self.read(|conn| {
            find_project(conn, root)?
                .map(|project| stack_of(conn, project))
                .transpose()
        })
    }

    fn stack_revision(&self, root: &ProjectRoot) -> Result<Option<StackRevision>, ConfigError> {
        self.read(|conn| {
            conn.query_row(
                "SELECT stack_revision FROM projects WHERE root = ?1",
                params![root_text(root)?],
                |row| row.get(0),
            )
            .optional()
            .map(|revision| revision.map(StackRevision::new))
            .map_err(db_error)
        })
    }
}

impl StackStore for SqliteStore {
    fn add_process(
        &self,
        root: &ProjectRoot,
        spec: &ProcessSpec,
        author: &Author,
    ) -> Result<Registration, ConfigError> {
        self.write(|conn| {
            spec.check_caps()?;
            let (project_id, registration) = register(conn, root)?;
            Stack::check_count(process_count(conn, project_id)? + 1)?;
            insert_process(conn, project_id, spec)?;
            bump_revision(conn, project_id)?;
            record_edit(conn, project_id, &StackEdit::Added {
                author: author.clone(),
                spec: spec.clone(),
            })?;
            Ok(registration)
        })
    }

    fn change_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
        author: &Author,
    ) -> Result<AppliedChange, ConfigError> {
        self.write(|conn| {
            let project = require_project(conn, root)?;
            let project_id = project.0;
            let before = stack_of(conn, project)?.require(name)?.clone();
            let changed = before.clone().with_change(change)?;
            let fields = before.differing_fields(&changed);
            if fields.is_empty() {
                return Ok(AppliedChange::builder()
                    .spec(changed)
                    .changed(false)
                    .build());
            }
            update_process(conn, project_id, &changed)?;
            bump_revision(conn, project_id)?;
            record_edit(conn, project_id, &StackEdit::Changed {
                author: author.clone(),
                fields,
                spec: changed.clone(),
            })?;
            Ok(AppliedChange::builder().spec(changed).changed(true).build())
        })
    }

    fn remove_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        author: &Author,
    ) -> Result<ProcessSpec, ConfigError> {
        self.write(|conn| {
            let project = require_project(conn, root)?;
            let project_id = project.0;
            let removed = stack_of(conn, project)?.require(name)?.clone();
            delete_process(conn, project_id, name)?;
            bump_revision(conn, project_id)?;
            record_edit(conn, project_id, &StackEdit::Removed {
                author: author.clone(),
                name: removed.name().clone(),
            })?;
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
            bump_revision(conn, project_id)?;
            Ok((registration, plan))
        })
    }

    fn set_idle_stop(
        &self,
        root: &ProjectRoot,
        idle_stop: Option<IdleStop>,
    ) -> Result<Project, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, root)?;
            store_idle_stop(conn, project_id, idle_stop.as_ref())?;
            bump_revision(conn, project_id)?;
            Ok(require_project(conn, root)?.1)
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
            config::{ENV_VARS_MAX, FieldEdit, OverCap, PROCESSES_MAX},
            port::{ProjectStore, SettingsReader, StackEditLog},
            process::{RestartPolicy, StopSignal},
            value::{ENV_VALUE_MAX_CHARS, PROCESS_NAME_MAX_CHARS, PositiveDuration},
        },
    };

    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    /// The `shop` folder under `scratch`, created, as a project root.
    fn root(scratch: &TempDir) -> ProjectRoot {
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap()
    }

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// A `web` process with every optional field set.
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
            .with_ports(BTreeSet::from([LocalPort::try_new(5173).unwrap()]))
    }

    /// The names of `stack`'s processes, in stack order.
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

    /// The readiness target stored for `process`, read straight from its row.
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

        store
            .add_process(&root, &full_spec(), &Author::human())
            .unwrap();
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();
        store
            .add_process(&root, &spec("db", "postgres"), &Author::human())
            .unwrap();

        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(names_of(&stack), ["web", "api", "db"]);
        assert_eq!(stack.processes()[0], full_spec());
        assert_eq!(stack.project().root(), &root);
    }

    /// A stored relative working directory that climbs out of the root, as a
    /// hand edit makes, leaves the project readable: that process reads as
    /// unconfined, its neighbour as stored, and a change or a removal of it
    /// lands.
    #[test]
    fn a_stored_working_dir_climbing_out_of_the_root_reads_unconfined() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &full_spec(), &Author::human())
            .unwrap();
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();
        let stored = |dir: &str| {
            store
                .write(|conn| {
                    conn.execute(
                        "UPDATE processes SET working_dir = ?1 WHERE name = 'web'",
                        params![dir],
                    )
                    .map_err(db_error)
                })
                .unwrap();
            store.stack(&root).unwrap().unwrap()
        };

        for escaping in ["../../../../etc", "a/../..", ""] {
            let stack = stored(escaping);
            let web = stack.process(&name("web")).unwrap();
            assert_eq!(
                web.unconfined_dir().map(|dir| dir.dir().clone()),
                Some(PathBuf::from(escaping))
            );
            assert_eq!(stack.process(&name("api")).unwrap().unconfined_dir(), None);
        }
        for reads in ["frontend/../src", "/srv/elsewhere"] {
            assert_eq!(
                stored(reads).processes()[0].unconfined_dir(),
                None,
                "{reads}"
            );
        }

        stored("../..");
        let cleared = ProcessChange::builder()
            .working_dir(FieldEdit::Clear)
            .build();
        let changed = store
            .change_process(&root, &name("web"), &cleared, &Author::human())
            .unwrap();
        assert!(changed.changed());
        assert_eq!(changed.spec().unconfined_dir(), None);
        stored("../..");
        store
            .remove_process(&root, &name("web"), &Author::human())
            .unwrap();
        assert_eq!(names_of(&store.stack(&root).unwrap().unwrap()), ["api"]);
    }

    /// Rows written before the caps, as raw SQL stands in for here, keep the
    /// project readable: a value past 8 KiB, more than the most variables
    /// and a name past the cap each read as stored with the cap named, the
    /// neighbour reads clean, an unrelated change and a repairing one land,
    /// and the over-long name is removed by that name.
    #[test]
    fn rows_stored_past_a_cap_read_and_repair() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        for process in ["web", "crowded", "api"] {
            store
                .add_process(&root, &spec(process, "true"), &Author::human())
                .unwrap();
        }
        let overlong = "n".repeat(PROCESS_NAME_MAX_CHARS + 1);
        store
            .write(|conn| {
                let id_of = |process: &str| -> Result<i64, ConfigError> {
                    conn.query_row(
                        "SELECT id FROM processes WHERE name = ?1",
                        params![process],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
                };
                let insert = |id: i64, var: &str, value: &str| {
                    conn.execute(
                        "INSERT INTO process_env (process_id, name, value) VALUES (?1, ?2, ?3)",
                        params![id, var, value],
                    )
                    .map_err(db_error)
                };
                insert(id_of("web")?, "CERT", &"v".repeat(ENV_VALUE_MAX_CHARS + 1))?;
                let crowded = id_of("crowded")?;
                for index in 0..=ENV_VARS_MAX {
                    insert(crowded, &format!("V{index}"), "1")?;
                }
                conn.execute(
                    "UPDATE processes SET name = ?1, name_key = ?1 WHERE name = 'api'",
                    params![overlong],
                )
                .map_err(db_error)
            })
            .unwrap();

        let stack = store.stack(&root).unwrap().unwrap();
        let over = |process: &str| stack.process(&name(process)).unwrap().over_cap();
        assert!(matches!(over("web"), Some(OverCap::VarValue { .. })));
        assert!(
            matches!(over("crowded"), Some(OverCap::Vars { found }) if found == ENV_VARS_MAX + 1)
        );
        assert!(matches!(over(&overlong), Some(OverCap::Name { .. })));

        let described = ProcessChange::builder()
            .description(FieldEdit::Set(Description::try_new("kept").unwrap()))
            .build();
        let kept = store
            .change_process(&root, &name("web"), &described, &Author::human())
            .unwrap();
        assert!(kept.changed());
        assert!(matches!(
            kept.spec().over_cap(),
            Some(OverCap::VarValue { .. })
        ));
        let unset = ProcessChange::builder()
            .unset_env(vec![EnvName::try_new("CERT").unwrap()])
            .build();
        let repaired = store
            .change_process(&root, &name("web"), &unset, &Author::human())
            .unwrap();
        assert_eq!(repaired.spec().over_cap(), None);
        let grow = ProcessChange::builder()
            .set_env(vec![(
                EnvName::try_new("EXTRA").unwrap(),
                EnvValue::try_new("1").unwrap(),
            )])
            .build();
        assert!(matches!(
            store.change_process(&root, &name("crowded"), &grow, &Author::human()),
            Err(ConfigError::OverCap { .. })
        ));
        store
            .remove_process(&root, &name(&overlong), &Author::human())
            .unwrap();
        assert_eq!(names_of(&store.stack(&root).unwrap().unwrap()), [
            "web", "crowded"
        ]);
    }

    /// A new process past a cap is refused, by an add or an import, naming
    /// the process and the cap, and nothing is written.
    #[test]
    fn a_new_process_past_a_cap_is_refused() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let long = spec("web", "true").with_environment(Environment::of([(
            EnvName::try_new("CERT").unwrap(),
            EnvValue::try_new("v".repeat(ENV_VALUE_MAX_CHARS + 1)).unwrap(),
        )]));

        let added = store
            .add_process(&root, &long, &Author::human())
            .unwrap_err();
        let imported = store.import(&root, &[long], ImportMode::Merge).unwrap_err();

        for refused in [added, imported] {
            assert!(
                refused
                    .to_string()
                    .starts_with("'web': variable 'CERT' is 8193 characters"),
                "{refused}"
            );
        }
        assert!(store.stack(&root).unwrap().is_none());
    }

    /// A hand-edited row a validator's own rule refuses names that rule in
    /// words when the stack is read.
    #[test]
    fn a_hand_edited_row_names_the_rule_it_breaks() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &full_spec(), &Author::human())
            .unwrap();
        let read_after = |sql: &str| {
            store
                .write(|conn| conn.execute_batch(sql).map_err(db_error))
                .unwrap();
            store.stack(&root).unwrap_err().to_string()
        };

        assert_eq!(
            read_after("UPDATE process_env SET name = 'MUSTER_' || name"),
            "stored variable name is invalid: it is empty, holds '=' or NUL, or starts with \
             MUSTER_"
        );
        assert_eq!(
            read_after("DELETE FROM process_env; UPDATE processes SET name = 'web' || char(7)"),
            "stored process name is invalid: it holds a control or format character"
        );
    }

    /// A declared port the schema check was bypassed for is refused on read
    /// with a sentence naming the rule.
    #[test]
    fn a_hand_edited_port_past_the_schema_names_the_rule() {
        for (port, reason) in [
            ("0", "it is not a port from 1 to 65535"),
            ("70000", "it is out of range"),
        ] {
            let (store, scratch) = fresh_store();
            let root = root(&scratch);
            store
                .add_process(&root, &full_spec(), &Author::human())
                .unwrap();
            store
                .write(|conn| {
                    conn.execute_batch(&format!(
                        "PRAGMA ignore_check_constraints = ON;
                         UPDATE process_ports SET port = {port};
                         PRAGMA ignore_check_constraints = OFF;"
                    ))
                    .map_err(db_error)
                })
                .unwrap();

            assert_eq!(
                store.stack(&root).unwrap_err().to_string(),
                format!("stored declared port is invalid: {reason}"),
                "{port}"
            );
        }
    }

    #[test]
    fn a_name_taken_by_nfc_key_is_refused() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &spec(CAFE_COMPOSED, "true"), &Author::human())
            .unwrap();

        let error = store
            .add_process(&root, &spec(CAFE_DECOMPOSED, "false"), &Author::human())
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
            .add_process(
                &root,
                &full_spec().with_ready(Some(port_check())),
                &Author::human(),
            )
            .unwrap();
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();
        let change = ProcessChange::builder()
            .command(Some(CommandLine::try_new("pnpm dev").unwrap()))
            .working_dir(FieldEdit::Clear)
            .unset_env(vec![EnvName::try_new("EMPTY").unwrap()])
            .stop_signal(Some(StopSignal::Interrupt))
            .build();

        let applied = store
            .change_process(&root, &name("web"), &change, &Author::human())
            .unwrap();
        let changed = applied.spec().clone();

        assert!(applied.changed());
        assert_eq!(
            changed,
            full_spec()
                .with_ready(Some(port_check()))
                .with_change(&change)
                .unwrap()
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
                &Author::human(),
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
                .change_process(&root, &name("web"), &set, &Author::human())
                .unwrap()
                .spec()
                .ready(),
            &Some(http)
        );

        let clear = ProcessChange::builder().ready(FieldEdit::Clear).build();
        store
            .change_process(&root, &name("web"), &clear, &Author::human())
            .unwrap();
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
            .timeout(PositiveDuration::try_new(Duration::from_millis(u64::MAX)).unwrap())
            .build();

        let error = store
            .add_process(
                &root,
                &spec("web", "npm run dev").with_ready(Some(endless)),
                &Author::human(),
            )
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
            store.change_process(&root, &name("web"), &change, &Author::human()),
            Err(ConfigError::UnknownProject { root: refused }) if &refused == root.as_ref()
        ));
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();

        for error in [
            store
                .change_process(&root, &name("web"), &change, &Author::human())
                .unwrap_err(),
            store
                .remove_process(&root, &name("web"), &Author::human())
                .unwrap_err(),
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
        store
            .add_process(&root, &full_spec(), &Author::human())
            .unwrap();

        let removed = store
            .remove_process(&root, &name("web"), &Author::human())
            .unwrap();

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
            .add_process(&root, &spec("web", "npm run dev"), &Author::human())
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

    /// A project holds at most the maximum processes: one more add, or an
    /// import that would leave more, is refused naming the count, and nothing
    /// is written; a replace that leaves fewer still lands.
    #[test]
    fn a_project_holds_at_most_the_maximum_processes() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let numbered = |index: usize| spec(&format!("p{index}"), "true");
        for index in 0..PROCESSES_MAX {
            store
                .add_process(&root, &numbered(index), &Author::human())
                .unwrap();
        }

        let added = store
            .add_process(&root, &numbered(PROCESSES_MAX), &Author::human())
            .unwrap_err();
        let imported = store
            .import(&root, &[numbered(PROCESSES_MAX)], ImportMode::Merge)
            .unwrap_err();

        for refused in [&added, &imported] {
            assert!(
                matches!(refused, ConfigError::TooManyProcesses { found } if *found == PROCESSES_MAX + 1),
                "{refused}"
            );
            assert!(refused.to_string().contains(&PROCESSES_MAX.to_string()));
        }
        assert_eq!(
            store.stack(&root).unwrap().unwrap().processes().len(),
            PROCESSES_MAX
        );
        store
            .import(&root, &[numbered(PROCESSES_MAX)], ImportMode::Replace)
            .unwrap();
        assert_eq!(names_of(&store.stack(&root).unwrap().unwrap()), [format!(
            "p{PROCESSES_MAX}"
        )]);
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
                &Author::human(),
            )
            .unwrap();
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();

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

    /// Every write to a stack moves its revision, and a project removed and
    /// registered again never comes back at a revision it had.
    #[test]
    fn every_stack_write_moves_the_revision() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        assert_eq!(store.stack_revision(&root).unwrap(), None);
        let mut seen = Vec::new();
        let mut moved = |store: &SqliteStore| {
            let revision = store.stack_revision(&root).unwrap().unwrap();
            assert!(!seen.contains(&revision), "{revision} repeated");
            seen.push(revision);
        };

        store
            .add_process(&root, &spec("web", "npm run dev"), &Author::human())
            .unwrap();
        moved(&store);
        store
            .change_process(
                &root,
                &name("web"),
                &ProcessChange::builder()
                    .working_dir(FieldEdit::Set(PathBuf::from("frontend")))
                    .build(),
                &Author::human(),
            )
            .unwrap();
        moved(&store);
        store
            .import(&root, &[spec("api", "cargo run")], ImportMode::Merge)
            .unwrap();
        moved(&store);
        store
            .remove_process(&root, &name("api"), &Author::human())
            .unwrap();
        moved(&store);
        assert!(store.remove_project(&root).unwrap());
        assert_eq!(store.stack_revision(&root).unwrap(), None);
        store
            .add_process(&root, &spec("web", "npm run dev"), &Author::human())
            .unwrap();
        moved(&store);
    }

    /// A change that leaves every value as it was writes nothing: the
    /// revision stays and the edit log keeps no entry; a change that also
    /// moves one field logs only that one.
    #[test]
    fn a_change_logs_only_what_it_changed() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let agent = Author::agent("claude");
        store
            .add_process(&root, &spec("web", "npm run dev"), &agent)
            .unwrap();
        let revision = store.stack_revision(&root).unwrap();
        let logged = store.edits(&root).unwrap().len();

        let same = ProcessChange::builder()
            .command(Some(CommandLine::try_new("npm run dev").unwrap()))
            .working_dir(FieldEdit::Clear)
            .build();
        let unchanged = store
            .change_process(&root, &name("web"), &same, &agent)
            .unwrap();
        assert!(!unchanged.changed());
        assert_eq!(store.stack_revision(&root).unwrap(), revision);
        assert_eq!(store.edits(&root).unwrap().len(), logged);

        let one = ProcessChange::builder()
            .command(Some(CommandLine::try_new("npm run dev").unwrap()))
            .autostart(Some(true))
            .build();
        assert!(
            store
                .change_process(&root, &name("web"), &one, &agent)
                .unwrap()
                .changed()
        );
        assert_ne!(store.stack_revision(&root).unwrap(), revision);
        assert_eq!(
            store.edits(&root).unwrap().last().unwrap().message(),
            "agent claude changed autostart of 'web'"
        );
    }

    /// The port `raw`.
    fn port(raw: u16) -> LocalPort {
        LocalPort::try_new(raw).unwrap()
    }

    /// The `process_ports` rows left in the database.
    fn port_rows(store: &SqliteStore) -> i64 {
        store
            .read(|conn| {
                conn.query_row("SELECT count(*) FROM process_ports", [], |row| row.get(0))
                    .map_err(db_error)
            })
            .unwrap()
    }

    /// A project's own idle stop is set, replaced and cleared, each write
    /// moving the revision; cleared, the project follows the setting.
    #[test]
    fn set_idle_stop_bumps_the_stack_revision_and_none_follows_the_setting() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        let five: IdleStop = "5m".parse().unwrap();

        for idle_stop in [Some(five), Some(IdleStop::Never), None] {
            let before = store.stack_revision(&root).unwrap();

            let project = store.set_idle_stop(&root, idle_stop).unwrap();

            assert_eq!(*project.idle_stop(), idle_stop);
            assert_eq!(store.project_at(&root).unwrap().unwrap(), project);
            assert_eq!(
                store.stack(&root).unwrap().unwrap().project(),
                &project,
                "the stack carries it"
            );
            assert_ne!(store.stack_revision(&root).unwrap(), before);
        }
        let setting = *store.load().unwrap().idle_stop();
        assert_eq!(
            store
                .project_at(&root)
                .unwrap()
                .unwrap()
                .idle_stop_or(setting),
            setting
        );
    }

    #[test]
    fn set_idle_stop_on_an_unregistered_root_is_unknown_project() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);

        let refused = store.set_idle_stop(&root, Some(IdleStop::Never));

        assert!(
            matches!(&refused, Err(ConfigError::UnknownProject { root: refused }) if refused == root.as_ref()),
            "{refused:?}"
        );
        assert_eq!(store.project_at(&root).unwrap(), None);
    }

    /// Declared ports read back ascending whatever order they were given in,
    /// and their rows go with their process, removed alone or with its
    /// project.
    #[test]
    fn ports_round_trip_in_order_and_go_with_their_process() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let ports = BTreeSet::from([port(5173), port(3000), port(8080)]);
        store
            .add_process(
                &root,
                &spec("web", "npm run dev").with_ports(ports.clone()),
                &Author::human(),
            )
            .unwrap();
        store
            .add_process(
                &root,
                &spec("api", "cargo run").with_ports(BTreeSet::from([port(3000)])),
                &Author::human(),
            )
            .unwrap();

        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(stack.processes()[0].ports(), &ports);
        assert_eq!(
            stack.processes()[0]
                .ports()
                .iter()
                .map(|port| port.into_inner())
                .collect::<Vec<_>>(),
            [3000, 5173, 8080]
        );
        assert_eq!(port_rows(&store), 4);

        store
            .remove_process(&root, &name("web"), &Author::human())
            .unwrap();
        assert_eq!(port_rows(&store), 1);
        assert!(store.remove_project(&root).unwrap());
        assert_eq!(port_rows(&store), 0);
    }

    /// A ports change replaces the rows and an empty set clears them, each
    /// logged as a change to the ports field; an import carries ports in.
    #[test]
    fn a_ports_change_replaces_the_rows_and_an_import_carries_ports() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        let agent = Author::agent("claude");
        store
            .add_process(
                &root,
                &spec("web", "npm run dev").with_ports(BTreeSet::from([port(3000), port(5173)])),
                &agent,
            )
            .unwrap();
        let ports_of_web = |store: &SqliteStore| {
            store.stack(&root).unwrap().unwrap().processes()[0]
                .ports()
                .clone()
        };

        let replace = ProcessChange::builder()
            .ports(Some(BTreeSet::from([port(8080)])))
            .build();
        store
            .change_process(&root, &name("web"), &replace, &agent)
            .unwrap();
        assert_eq!(ports_of_web(&store), BTreeSet::from([port(8080)]));
        assert_eq!(port_rows(&store), 1);
        assert_eq!(
            store.edits(&root).unwrap().last().unwrap().message(),
            "agent claude changed ports of 'web'"
        );

        let clear = ProcessChange::builder()
            .ports(Some(BTreeSet::new()))
            .build();
        store
            .change_process(&root, &name("web"), &clear, &agent)
            .unwrap();
        assert!(ports_of_web(&store).is_empty());
        assert_eq!(port_rows(&store), 0);

        let imported = spec("web", "npm run dev").with_ports(BTreeSet::from([port(4000)]));
        let added = spec("worker", "bin/worker").with_ports(BTreeSet::from([port(9000)]));
        store
            .import(&root, &[imported.clone(), added.clone()], ImportMode::Merge)
            .unwrap();
        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(stack.processes(), &vec![imported, added]);
        assert_eq!(port_rows(&store), 2);
    }

    #[test]
    fn a_replace_import_leaves_exactly_the_file() {
        let (store, scratch) = fresh_store();
        let root = root(&scratch);
        store
            .add_process(&root, &spec("web", "npm run dev"), &Author::human())
            .unwrap();
        store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();

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
