use std::time::UNIX_EPOCH;

use rusqlite::{Row, params};

use super::{
    project_rows::find_project,
    sqlite_store::{
        SqliteStore, column_integer, corrupt, db_error, duration_from_millis, duration_millis,
        refused,
    },
};
use crate::domain::{
    config::ConfigError,
    port::RunHistory,
    process::EndedRun,
    pty::{ExitCause, ExitOutcome, SignalName},
    value::{ProcessName, ProjectRoot},
};

/// The column naming the process a run belongs to.
const NAME: &str = "name";
/// The column holding an exit status.
const EXIT_STATUS: &str = "exit_status";
/// The column holding the signal that ended a run.
const EXIT_SIGNAL: &str = "exit_signal";
/// The column holding when a run ended, in Unix milliseconds.
const ENDED_AT_MS: &str = "ended_at_ms";
/// What a run's end time is called in a refusal.
const ENDED_AT: &str = "run end time";

/// Reads one `process_runs` row joined to its process's name.
fn run_from_row(row: &Row) -> Result<EndedRun, ConfigError> {
    let name: String = row.get(NAME).map_err(db_error)?;
    let status: Option<i64> = row.get(EXIT_STATUS).map_err(db_error)?;
    let signal: Option<String> = row.get(EXIT_SIGNAL).map_err(db_error)?;
    let ended_ms: i64 = row.get(ENDED_AT_MS).map_err(db_error)?;
    let outcome = match (status, signal) {
        (Some(0), None) => ExitOutcome::Succeeded,
        (Some(status), None) => ExitOutcome::Failed(ExitCause::Status(
            u32::try_from(status).map_err(|error| refused(EXIT_STATUS, &error))?,
        )),
        (None, Some(signal)) => ExitOutcome::Failed(ExitCause::Signal(
            SignalName::try_new(signal).map_err(|error| refused(EXIT_SIGNAL, &error))?,
        )),
        (None, None) => ExitOutcome::FAILED_UNKNOWN,
        (Some(_), Some(_)) => return Err(corrupt(EXIT_SIGNAL, "a status and a signal")),
    };
    Ok(EndedRun::builder()
        .process(ProcessName::try_new(name).map_err(|error| refused(NAME, &error))?)
        .outcome(outcome)
        .at(UNIX_EPOCH + duration_from_millis(ENDED_AT, ended_ms)?)
        .build())
}

/// The status and signal columns `outcome` is stored as.
fn outcome_columns(outcome: &ExitOutcome) -> (Option<u32>, Option<&str>) {
    match outcome {
        ExitOutcome::Succeeded => (Some(0), None),
        ExitOutcome::Failed(ExitCause::Status(status)) => (Some(*status), None),
        ExitOutcome::Failed(ExitCause::Signal(signal)) => (None, Some(signal.as_ref())),
        ExitOutcome::Failed(ExitCause::Unknown) => (None, None),
    }
}

impl RunHistory for SqliteStore {
    fn ended_runs(&self, root: &ProjectRoot) -> Result<Vec<EndedRun>, ConfigError> {
        self.read(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(Vec::new());
            };
            let mut statement = conn
                .prepare(
                    "SELECT processes.name, exit_status, exit_signal, ended_at_ms \
                     FROM process_runs JOIN processes ON processes.id = process_runs.process_id \
                     WHERE processes.project_id = ?1 ORDER BY processes.id",
                )
                .map_err(db_error)?;
            let mut rows = statement.query(params![project_id]).map_err(db_error)?;
            let mut runs = Vec::new();
            while let Some(row) = rows.next().map_err(db_error)? {
                runs.push(run_from_row(row)?);
            }
            Ok(runs)
        })
    }

    fn record_end(&self, root: &ProjectRoot, run: &EndedRun) -> Result<(), ConfigError> {
        let (status, signal) = outcome_columns(run.outcome());
        let ended_ms = duration_millis(
            ENDED_AT,
            run.at().duration_since(UNIX_EPOCH).unwrap_or_default(),
        )?;
        self.write(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(());
            };
            conn.execute(
                "INSERT INTO process_runs (process_id, exit_status, exit_signal, ended_at_ms) \
                 SELECT id, ?3, ?4, ?5 FROM processes WHERE project_id = ?1 AND name_key = ?2 \
                 ON CONFLICT (process_id) DO UPDATE SET exit_status = excluded.exit_status, \
                 exit_signal = excluded.exit_signal, ended_at_ms = excluded.ended_at_ms",
                params![
                    project_id,
                    run.process().key(),
                    status
                        .map(|status| column_integer(EXIT_STATUS, status))
                        .transpose()?,
                    signal,
                    ended_ms
                ],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }

    fn clear_end(&self, root: &ProjectRoot, process: &ProcessName) -> Result<(), ConfigError> {
        self.write(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(());
            };
            conn.execute(
                "DELETE FROM process_runs WHERE process_id IN \
                 (SELECT id FROM processes WHERE project_id = ?1 AND name_key = ?2)",
                params![project_id, process.key()],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    use super::*;
    use crate::{
        adapter::store::test_support::{fresh_store, spec},
        domain::{coordination::Author, port::StackStore},
    };

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// The run of `process` that ended with `outcome`, at a fixed moment.
    fn ended(process: &str, outcome: ExitOutcome) -> EndedRun {
        EndedRun::builder()
            .process(name(process))
            .outcome(outcome)
            .at(UNIX_EPOCH + Duration::from_millis(1_700_000_000_123))
            .build()
    }

    /// Every exit shape reads back as it was recorded, a newer end replaces
    /// the older, and a cleared or removed process has none.
    #[test]
    fn an_ended_run_reads_back_until_cleared_or_removed() {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir).unwrap();
        let human = Author::human();
        for process in ["api", "web", "worker", "db"] {
            store
                .add_process(&root, &spec(process, "true"), &human)
                .unwrap();
        }
        let signal = SignalName::try_new("sigkill").unwrap();
        let runs = [
            ended("api", ExitOutcome::Failed(ExitCause::Status(3))),
            ended("web", ExitOutcome::Failed(ExitCause::Signal(signal))),
            ended("worker", ExitOutcome::Succeeded),
            ended("db", ExitOutcome::FAILED_UNKNOWN),
        ];
        store
            .record_end(&root, &ended("api", ExitOutcome::Succeeded))
            .unwrap();
        for run in &runs {
            store.record_end(&root, run).unwrap();
        }

        assert_eq!(store.ended_runs(&root).unwrap(), runs);

        store.clear_end(&root, &name("api")).unwrap();
        store.remove_process(&root, &name("web"), &human).unwrap();
        let left: Vec<String> = store
            .ended_runs(&root)
            .unwrap()
            .iter()
            .map(|run| run.process().to_string())
            .collect();
        assert_eq!(left, ["worker", "db"]);
        assert!(*store.ended_runs(&root).unwrap()[0].at() <= SystemTime::now());
    }
}
