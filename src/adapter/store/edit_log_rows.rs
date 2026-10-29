use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Row, params};

use super::{
    project_rows::{find_project, require_project},
    sqlite_store::{
        SqliteStore, column_integer, corrupt, db_error, duration_from_millis, duration_millis,
    },
};
use crate::domain::{
    config::{ConfigError, RecordedEdit, StackEdit},
    port::StackEditLog,
    value::{ProcessName, ProjectRoot},
};

/// Most edits kept per project; recording one more drops the oldest.
const EDIT_LOG_RETENTION: usize = 50;
/// The column holding when an edit was recorded, in Unix milliseconds.
const AT_MS: &str = "at_ms";
/// The column naming the process an edit concerns.
const PROCESS: &str = "process";
/// The column holding an edit's text.
const MESSAGE: &str = "message";
/// What a recorded time is called in a refusal.
const RECORDED_AT: &str = "edit time";
/// What the retention bound is called in a refusal.
const RETENTION: &str = "edit log retention";

/// Reads one `stack_edits` row.
fn edit_from_row(row: &Row) -> Result<RecordedEdit, ConfigError> {
    let at_ms: i64 = row.get(AT_MS).map_err(db_error)?;
    let process: String = row.get(PROCESS).map_err(db_error)?;
    Ok(RecordedEdit::builder()
        .at(UNIX_EPOCH + duration_from_millis(RECORDED_AT, at_ms)?)
        .process(ProcessName::try_new(process).map_err(|error| corrupt(PROCESS, error))?)
        .message(row.get(MESSAGE).map_err(db_error)?)
        .build())
}

/// Milliseconds since the Unix epoch now, or zero if the clock predates it.
///
/// # Errors
/// Returns [`ConfigError::TooLarge`] when the time does not fit a column.
fn now_millis() -> Result<i64, ConfigError> {
    duration_millis(
        RECORDED_AT,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

impl StackEditLog for SqliteStore {
    fn record_edit(&self, root: &ProjectRoot, edit: &StackEdit) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, root)?;
            conn.execute(
                "INSERT INTO stack_edits (project_id, at_ms, process, message) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    project_id,
                    now_millis()?,
                    edit.process().as_ref(),
                    edit.to_string()
                ],
            )
            .map_err(db_error)?;
            conn.execute(
                "DELETE FROM stack_edits WHERE project_id = ?1 AND id NOT IN \
                 (SELECT id FROM stack_edits WHERE project_id = ?1 ORDER BY id DESC LIMIT ?2)",
                params![project_id, column_integer(RETENTION, EDIT_LOG_RETENTION)?],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }

    fn edits(&self, root: &ProjectRoot) -> Result<Vec<RecordedEdit>, ConfigError> {
        self.read(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(Vec::new());
            };
            let mut statement = conn
                .prepare(
                    "SELECT at_ms, process, message FROM stack_edits WHERE project_id = ?1 \
                     ORDER BY id",
                )
                .map_err(db_error)?;
            let mut rows = statement.query(params![project_id]).map_err(db_error)?;
            let mut edits = Vec::new();
            while let Some(row) = rows.next().map_err(db_error)? {
                edits.push(edit_from_row(row)?);
            }
            Ok(edits)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{fresh_store, spec},
        domain::{
            coordination::Author,
            port::{ProjectStore, StackStore},
        },
    };

    fn removal(name: &str) -> StackEdit {
        StackEdit::Removed {
            author: Author::agent("claude"),
            name: ProcessName::try_new(name).unwrap(),
        }
    }

    fn project(store: &SqliteStore, scratch: &TempDir) -> ProjectRoot {
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir).unwrap();
        store.add_process(&root, &spec("web", "true")).unwrap();
        root
    }

    /// An edit is kept with its process and text, oldest first.
    #[test]
    fn a_recorded_edit_reads_back_in_order() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);
        let before = SystemTime::now();

        store.record_edit(&root, &removal("web")).unwrap();
        store.record_edit(&root, &removal("api")).unwrap();

        let edits = store.edits(&root).unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].process().as_ref(), "web");
        assert_eq!(edits[0].message(), &removal("web").to_string());
        assert_eq!(edits[1].process().as_ref(), "api");
        assert!(*edits[0].at() >= before);
    }

    /// Only the newest [`EDIT_LOG_RETENTION`] edits survive.
    #[test]
    fn the_log_keeps_only_the_newest_edits() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);

        for index in 0..=EDIT_LOG_RETENTION {
            store
                .record_edit(&root, &removal(&format!("p{index}")))
                .unwrap();
        }

        let edits = store.edits(&root).unwrap();
        assert_eq!(edits.len(), EDIT_LOG_RETENTION);
        assert_eq!(edits[0].process().as_ref(), "p1");
        assert_eq!(
            edits.last().unwrap().process().as_ref(),
            format!("p{EDIT_LOG_RETENTION}")
        );
    }

    /// An unregistered folder has no log and cannot be written to, and
    /// removing a project removes its log.
    #[test]
    fn the_log_belongs_to_a_registered_project() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);
        let elsewhere = ProjectRoot::try_new(scratch.path().join("other")).unwrap();

        assert!(store.edits(&elsewhere).unwrap().is_empty());
        assert!(matches!(
            store.record_edit(&elsewhere, &removal("web")),
            Err(ConfigError::UnknownProject { .. })
        ));

        store.record_edit(&root, &removal("web")).unwrap();
        store.remove_project(&root).unwrap();
        store.add_process(&root, &spec("web", "true")).unwrap();
        assert!(store.edits(&root).unwrap().is_empty());
    }
}
