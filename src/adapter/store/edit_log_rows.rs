use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Row, params};

use super::{
    project_rows::find_project,
    sqlite_store::{
        SqliteStore, column_integer, corrupt, db_error, duration_from_millis, duration_millis,
    },
};
use crate::domain::{
    config::{ConfigError, RecordedEdit, StackEdit},
    port::StackEditLog,
    value::{EditId, ProcessName, ProjectRoot},
};

/// Most edits kept per project; recording one more drops the oldest.
const EDIT_LOG_RETENTION: usize = 50;
/// The column holding an edit's entry id.
const ID: &str = "id";
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
        .id(EditId::new(row.get(ID).map_err(db_error)?))
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

/// Records `edit` to the log of the project `project_id`, stamped now,
/// inside the caller's write transaction when the log keeps it, and drops
/// the entries past [`EDIT_LOG_RETENTION`].
///
/// # Errors
/// Returns a storage failure when the log cannot be written, which rolls
/// the caller's transaction back.
pub(super) fn record_edit(
    conn: &Connection,
    project_id: i64,
    edit: &StackEdit,
) -> Result<(), ConfigError> {
    if !edit.is_logged() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO stack_edits (project_id, at_ms, process, message) VALUES (?1, ?2, ?3, ?4)",
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
}

impl StackEditLog for SqliteStore {
    fn edits(&self, root: &ProjectRoot) -> Result<Vec<RecordedEdit>, ConfigError> {
        self.read(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(Vec::new());
            };
            let mut statement = conn
                .prepare(
                    "SELECT id, at_ms, process, message FROM stack_edits WHERE project_id = ?1 \
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
            config::ProcessChange,
            coordination::Author,
            port::{ProjectStore, StackReader, StackStore},
        },
    };

    /// Makes every later write to the edit log fail, as a crash between the
    /// stack write and its log entry would.
    const REFUSE_EDITS: &str = "CREATE TRIGGER refuse_edits BEFORE INSERT ON stack_edits \
                                BEGIN SELECT RAISE(ABORT, 'refused'); END;";

    fn agent() -> Author {
        Author::agent("claude")
    }

    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    fn folder(scratch: &TempDir) -> ProjectRoot {
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        ProjectRoot::try_new(dir).unwrap()
    }

    /// The `shop` project with a `web` process the human added.
    fn project(store: &SqliteStore, scratch: &TempDir) -> ProjectRoot {
        let root = folder(scratch);
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        root
    }

    fn removal(raw: &str) -> String {
        StackEdit::Removed {
            author: agent(),
            name: name(raw),
        }
        .to_string()
    }

    /// An agent's add, change and remove are each kept with their process
    /// and text, oldest first, with rising ids.
    #[test]
    fn an_agent_edit_is_recorded_by_the_write_that_applies_it() {
        let (store, scratch) = fresh_store();
        let root = folder(&scratch);
        let before = SystemTime::now();

        let added = spec("web", "true");
        store.add_process(&root, &added, &agent()).unwrap();
        let change = ProcessChange::builder().autostart(Some(true)).build();
        let changed = store
            .change_process(&root, &name("web"), &change, &agent())
            .unwrap();
        store.remove_process(&root, &name("web"), &agent()).unwrap();

        let edits = store.edits(&root).unwrap();
        let messages: Vec<&String> = edits.iter().map(RecordedEdit::message).collect();
        assert_eq!(messages, [
            &StackEdit::Added {
                author: agent(),
                spec: added,
            }
            .to_string(),
            &StackEdit::Changed {
                author: agent(),
                fields: change.fields(),
                spec: changed,
            }
            .to_string(),
            &removal("web"),
        ]);
        assert!(edits.iter().all(|edit| edit.process().as_ref() == "web"));
        assert!(edits[0].id() < edits[1].id() && edits[1].id() < edits[2].id());
        assert!(*edits[0].at() >= before);
    }

    /// The human's edits at the CLI are never kept in the log.
    #[test]
    fn a_human_edit_is_not_recorded() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);

        store
            .change_process(
                &root,
                &name("web"),
                &ProcessChange::builder().autostart(Some(true)).build(),
                &Author::human(),
            )
            .unwrap();
        store
            .remove_process(&root, &name("web"), &Author::human())
            .unwrap();

        assert!(store.edits(&root).unwrap().is_empty());
    }

    /// When the log entry cannot be written, the stack edit it records is
    /// rolled back with it: neither the process nor the revision moves.
    #[test]
    fn an_edit_whose_log_entry_fails_is_not_applied() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);
        let revision = store.stack_revision(&root).unwrap();
        store
            .write(|conn| conn.execute_batch(REFUSE_EDITS).map_err(db_error))
            .unwrap();

        assert!(
            store
                .add_process(&root, &spec("api", "true"), &agent())
                .is_err()
        );
        assert!(
            store
                .change_process(
                    &root,
                    &name("web"),
                    &ProcessChange::builder().autostart(Some(true)).build(),
                    &agent(),
                )
                .is_err()
        );
        assert!(store.remove_process(&root, &name("web"), &agent()).is_err());

        let stack = store.stack(&root).unwrap().unwrap();
        assert_eq!(stack.names(), [name("web")]);
        assert!(!*stack.require(&name("web")).unwrap().autostart());
        assert_eq!(store.stack_revision(&root).unwrap(), revision);
        assert!(store.edits(&root).unwrap().is_empty());
    }

    /// Only the newest [`EDIT_LOG_RETENTION`] edits survive.
    #[test]
    fn the_log_keeps_only_the_newest_edits() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);

        for index in 0..=EDIT_LOG_RETENTION {
            let process = format!("p{index}");
            store
                .add_process(&root, &spec(&process, "true"), &Author::human())
                .unwrap();
            store
                .remove_process(&root, &name(&process), &agent())
                .unwrap();
        }

        let edits = store.edits(&root).unwrap();
        assert_eq!(edits.len(), EDIT_LOG_RETENTION);
        assert_eq!(edits[0].message(), &removal("p1"));
        assert_eq!(
            edits.last().unwrap().message(),
            &removal(&format!("p{EDIT_LOG_RETENTION}"))
        );
    }

    /// An unregistered folder has no log, and removing a project removes its
    /// log.
    #[test]
    fn the_log_belongs_to_a_registered_project() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);
        let elsewhere = ProjectRoot::try_new(scratch.path().join("other")).unwrap();

        assert!(store.edits(&elsewhere).unwrap().is_empty());

        store.remove_process(&root, &name("web"), &agent()).unwrap();
        store.remove_project(&root).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        assert!(store.edits(&root).unwrap().is_empty());
    }
}
