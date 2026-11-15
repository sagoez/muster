use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use muster_domain::{
    config::{Environment, ProcessSpec},
    process::{RestartPolicy, StopPolicy, StopSignal},
    value::{CommandLine, MAX_STOP_GRACE, ProcessName, ProjectRoot, StopGrace},
};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use super::{DATABASE_FILE, SqliteStore};

/// The state folder a test database lives in, under its scratch directory.
const STATE_DIR: &str = "state";
/// The grace period test processes stop with.
const TEST_GRACE: Duration = Duration::from_secs(5);
/// A stop grace past the longest a stack may hold: the schema stores it,
/// the reader refuses it.
const REFUSED_GRACE: Duration = MAX_STOP_GRACE.saturating_mul(2);

/// Corrupts every process row of the project at `root` in the database at
/// `database` the way a hand edit or another muster might: in a form the
/// schema allows but the reader refuses, so that project's stack no longer
/// reads and every other one still does.
pub fn corrupt_processes(database: &Path, root: &ProjectRoot) {
    let grace = i64::try_from(REFUSED_GRACE.as_millis()).unwrap();
    let changed = Connection::open(database)
        .unwrap()
        .execute(
            "UPDATE processes SET stop_grace_ms = ?1 \
             WHERE project_id = (SELECT id FROM projects WHERE root = ?2)",
            params![grace, root.as_ref().to_string_lossy()],
        )
        .unwrap();
    assert!(
        changed > 0,
        "no process of {} to corrupt",
        root.as_ref().display()
    );
}

/// Where [`fresh_store`] keeps its database inside `scratch`.
pub fn database(scratch: &TempDir) -> PathBuf {
    scratch.path().join(STATE_DIR).join(DATABASE_FILE)
}

/// A store over a new database in a scratch directory, which also holds any
/// project folders the test makes.
pub fn fresh_store() -> (SqliteStore, TempDir) {
    let scratch = TempDir::new().unwrap();
    let store = SqliteStore::open(&database(&scratch)).unwrap();
    (store, scratch)
}

/// A plain definition named `name` running `command`.
pub fn spec(name: &str, command: &str) -> ProcessSpec {
    ProcessSpec::builder()
        .name(ProcessName::try_new(name).unwrap())
        .command(CommandLine::try_new(command).unwrap())
        .environment(Environment::empty())
        .autostart(false)
        .restart(RestartPolicy::Never)
        .stop(
            StopPolicy::builder()
                .signal(StopSignal::Terminate)
                .grace_period(StopGrace::try_new(TEST_GRACE).unwrap())
                .build(),
        )
        .build()
}
