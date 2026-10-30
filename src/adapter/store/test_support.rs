use std::{path::PathBuf, time::Duration};

use tempfile::TempDir;

use super::{DATABASE_FILE, SqliteStore};
use crate::domain::{
    config::{Environment, ProcessSpec},
    process::{RestartPolicy, StopPolicy, StopSignal},
    value::{CommandLine, ProcessName, StopGrace},
};

/// The state folder a test database lives in, under its scratch directory.
const STATE_DIR: &str = "state";
/// The grace period test processes stop with.
const TEST_GRACE: Duration = Duration::from_secs(5);

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
