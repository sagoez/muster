use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{
    project_rows::find_project,
    sqlite_store::{
        SqliteStore, column_integer, db_error, duration_from_millis, duration_millis, refused,
    },
};
use crate::domain::{
    config::{ConfigError, RecordedEdit, StackEdit},
    port::{RaisedEdits, StackEditLog},
    value::{EditId, ProcessName, ProcessNameError, ProjectRoot, elided_name, printable},
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

/// Reads one `stack_edits` row, its text made [`printable`] since it quotes
/// a command an agent wrote.
fn edit_from_row(row: &Row) -> Result<RecordedEdit, ConfigError> {
    let at_ms: i64 = row.get(AT_MS).map_err(db_error)?;
    let process: Option<String> = row.get(PROCESS).map_err(db_error)?;
    let message: String = row.get(MESSAGE).map_err(db_error)?;
    Ok(RecordedEdit::builder()
        .id(EditId::new(row.get(ID).map_err(db_error)?))
        .at(UNIX_EPOCH + duration_from_millis(RECORDED_AT, at_ms)?)
        .process(process.map(edited_process).transpose()?)
        .message(printable(&message))
        .build())
}

/// The process an edit names, from its stored `raw` name. A name stored before
/// the name cap bounded it reads as its [`elided_name`], so an
/// old entry never makes the log unreadable; any other refused name is corrupt.
fn edited_process(raw: String) -> Result<ProcessName, ConfigError> {
    match ProcessName::try_new(raw.as_str()) {
        Err(ProcessNameError::LenCharMaxViolated) => {
            elided_name(&raw).map_err(|error| refused(PROCESS, &error))
        },
        read => read.map_err(|error| refused(PROCESS, &error)),
    }
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
            edit.process().map(AsRef::<str>::as_ref),
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

/// Records `edit` to the log of every registered project, as
/// [`record_edit`] does for one.
///
/// # Errors
/// Returns a storage failure when a log cannot be written, which rolls the
/// caller's transaction back.
pub(super) fn record_edit_everywhere(
    conn: &Connection,
    edit: &StackEdit,
) -> Result<(), ConfigError> {
    if !edit.is_logged() {
        return Ok(());
    }
    let mut statement = conn
        .prepare("SELECT id FROM projects ORDER BY id")
        .map_err(db_error)?;
    let ids: Vec<i64> = statement
        .query_map([], |row| row.get(0))
        .map_err(db_error)?
        .collect::<Result<_, _>>()
        .map_err(db_error)?;
    for project_id in ids {
        record_edit(conn, project_id, edit)?;
    }
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

impl RaisedEdits for SqliteStore {
    fn raised_through(&self, root: &ProjectRoot) -> Result<Option<EditId>, ConfigError> {
        self.read(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(None);
            };
            conn.query_row(
                "SELECT edit_id FROM raised_edits WHERE project_id = ?1",
                params![project_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(db_error)
            .map(|edit| edit.map(EditId::new))
        })
    }

    fn mark_raised(&self, root: &ProjectRoot, edit: EditId) -> Result<(), ConfigError> {
        self.write(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(());
            };
            conn.execute(
                "INSERT INTO raised_edits (project_id, edit_id) VALUES (?1, ?2) \
                 ON CONFLICT (project_id) DO UPDATE SET edit_id = max(edit_id, excluded.edit_id)",
                params![project_id, edit.into_inner()],
            )
            .map_err(db_error)?;
            Ok(())
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
            value::PROCESS_NAME_MAX_CHARS,
        },
    };

    /// Makes every later write to the edit log fail, as a crash between the
    /// stack write and its log entry would.
    const REFUSE_EDITS: &str = "CREATE TRIGGER refuse_edits BEFORE INSERT ON stack_edits \
                                BEGIN SELECT RAISE(ABORT, 'refused'); END;";

    /// The agent every logged edit is made by.
    fn agent() -> Author {
        Author::agent("claude")
    }

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// The `shop` folder under `scratch`, created.
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

    /// How the log reads the agent removing the process `raw`.
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
            .unwrap()
            .spec()
            .clone();
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
        assert!(
            edits
                .iter()
                .all(|edit| edit.process().as_ref().map(AsRef::as_ref) == Some("web"))
        );
        assert!(edits[0].id() < edits[1].id() && edits[1].id() < edits[2].id());
        assert!(*edits[0].at() >= before);
    }

    /// An agent's command carrying an escape reads back from the log as
    /// text, so a notice quoting it cannot drive the terminal it is shown on.
    #[test]
    fn an_edit_reads_back_escaped() {
        let (store, scratch) = fresh_store();
        let root = folder(&scratch);

        store
            .add_process(&root, &spec("web", "echo \u{1b}]0;owned\u{7}"), &agent())
            .unwrap();

        let message = store.edits(&root).unwrap()[0].message().clone();
        assert!(!message.contains('\u{1b}') && !message.contains('\u{7}'));
        assert!(message.contains("\\u{1b}]0;owned\\u{7}"), "{message}");
    }

    /// An entry naming a process past the cap, kept from before the cap,
    /// reads back with that name escaped and cut to the cap, while a process
    /// row holding such a name is still refused.
    #[test]
    fn an_edit_naming_a_process_past_the_cap_still_reads() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);
        store
            .add_process(&root, &spec("api", "true"), &agent())
            .unwrap();
        let overlong = format!("{}\u{1b}[2J", "n".repeat(PROCESS_NAME_MAX_CHARS));
        store
            .write(|conn| {
                conn.execute("UPDATE stack_edits SET process = ?1", params![overlong])
                    .map_err(db_error)
            })
            .unwrap();

        let edits = store.edits(&root).unwrap();

        let shown = edits[0].process().clone().unwrap();
        assert_eq!(shown, elided_name(&overlong).unwrap());
        assert_eq!(shown.as_ref().chars().count(), PROCESS_NAME_MAX_CHARS);
        assert!(!shown.as_ref().contains('\u{1b}'), "{shown}");

        store
            .write(|conn| {
                conn.execute(
                    "UPDATE processes SET name = ?1 WHERE name = 'api'",
                    params![overlong],
                )
                .map_err(db_error)
            })
            .unwrap();
        assert_eq!(
            store.stack(&root).unwrap_err().to_string(),
            "stored process name is invalid: it is longer than muster allows"
        );
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

    /// The raised mark only moves forward, and a project registered again
    /// starts without one.
    #[test]
    fn the_raised_mark_only_moves_forward() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch);

        assert_eq!(store.raised_through(&root).unwrap(), None);
        store.mark_raised(&root, EditId::new(4)).unwrap();
        store.mark_raised(&root, EditId::new(2)).unwrap();
        assert_eq!(store.raised_through(&root).unwrap(), Some(EditId::new(4)));

        store.remove_project(&root).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        assert_eq!(store.raised_through(&root).unwrap(), None);
    }
}
