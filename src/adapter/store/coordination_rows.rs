use rusqlite::{Connection, Params, Row, params};

use super::{
    project_rows::require_project,
    sqlite_store::{SqliteStore, corrupt, db_error, now_epoch_secs},
};
use crate::domain::{
    config::ConfigError,
    coordination::{
        Author, KeyValue, KvKey, Scratchpad, ScratchpadKey, Todo, TodoId, TodoStatus, TodoTitle,
    },
    port::CoordinationStore,
    value::ProjectRoot,
};

/// Selects a scratchpad in the order [`scratchpad_from_row`] reads it.
const SELECT_SCRATCHPAD: &str = "SELECT key, body, author, updated_at FROM scratchpads";
/// Selects a todo in the order [`todo_from_row`] reads it.
const SELECT_TODO: &str = "SELECT id, title, status, deps, author, updated_at FROM todos";
/// Selects a key-value in the order [`key_value_from_row`] reads it.
const SELECT_KV: &str = "SELECT key, value, author, updated_at FROM kv";

/// A stored write time, which the store only ever stamps from the clock.
fn updated_at(row: &Row, index: usize) -> Result<u64, ConfigError> {
    let stamp: i64 = row.get(index).map_err(db_error)?;
    u64::try_from(stamp).map_err(|error| corrupt("write time", error))
}

/// Serializes a todo's dependency ids to the JSON stored in `deps`.
fn deps_json(deps: &[TodoId]) -> Result<String, ConfigError> {
    serde_json::to_string(deps).map_err(|error| ConfigError::Database(error.to_string()))
}

/// Reads a [`SELECT_SCRATCHPAD`] row.
fn scratchpad_from_row(row: &Row) -> Result<Scratchpad, ConfigError> {
    let key: String = row.get(0).map_err(db_error)?;
    let body: String = row.get(1).map_err(db_error)?;
    let author: String = row.get(2).map_err(db_error)?;
    Ok(Scratchpad::builder()
        .key(ScratchpadKey::try_new(key).map_err(|error| corrupt("scratchpad key", error))?)
        .body(body)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row, 3)?)
        .build())
}

/// Reads a [`SELECT_TODO`] row.
fn todo_from_row(row: &Row) -> Result<Todo, ConfigError> {
    let id: String = row.get(0).map_err(db_error)?;
    let title: String = row.get(1).map_err(db_error)?;
    let status: String = row.get(2).map_err(db_error)?;
    let deps: String = row.get(3).map_err(db_error)?;
    let author: String = row.get(4).map_err(db_error)?;
    Ok(Todo::builder()
        .id(TodoId::try_new(id).map_err(|error| corrupt("todo id", error))?)
        .title(TodoTitle::try_new(title).map_err(|error| corrupt("todo title", error))?)
        .status(
            status
                .parse()
                .map_err(|error| corrupt("todo status", error))?,
        )
        .deps(serde_json::from_str(&deps).map_err(|error| corrupt("todo deps", error))?)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row, 5)?)
        .build())
}

/// Reads a [`SELECT_KV`] row.
fn key_value_from_row(row: &Row) -> Result<KeyValue, ConfigError> {
    let key: String = row.get(0).map_err(db_error)?;
    let value: String = row.get(1).map_err(db_error)?;
    let author: String = row.get(2).map_err(db_error)?;
    Ok(KeyValue::builder()
        .key(KvKey::try_new(key).map_err(|error| corrupt("kv key", error))?)
        .value(value)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row, 3)?)
        .build())
}

/// Every row `sql` selects with `values`, mapped with `map`.
fn collect<T>(
    conn: &Connection,
    sql: &str,
    values: impl Params,
    map: impl Fn(&Row) -> Result<T, ConfigError>,
) -> Result<Vec<T>, ConfigError> {
    let mut statement = conn.prepare(sql).map_err(db_error)?;
    let mut rows = statement.query(values).map_err(db_error)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        out.push(map(row)?);
    }
    Ok(out)
}

/// Writes a scratchpad with the author and time it carries.
fn put_scratchpad(
    conn: &Connection,
    project_id: i64,
    note: &Scratchpad,
) -> Result<(), ConfigError> {
    conn.execute(
        "INSERT OR REPLACE INTO scratchpads (project_id, key, body, author, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            project_id,
            note.key().as_ref(),
            note.body(),
            note.author().label(),
            *note.updated_at() as i64
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Writes a todo with the author and time it carries; it is not claimed.
fn put_todo(conn: &Connection, project_id: i64, todo: &Todo) -> Result<(), ConfigError> {
    conn.execute(
        "INSERT OR REPLACE INTO todos (project_id, id, title, status, deps, author, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            project_id,
            todo.id().as_ref(),
            todo.title().as_ref(),
            todo.status().to_string(),
            deps_json(todo.deps())?,
            todo.author().label(),
            *todo.updated_at() as i64
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Writes a key-value with the author and time it carries.
fn put_value(conn: &Connection, project_id: i64, entry: &KeyValue) -> Result<(), ConfigError> {
    conn.execute(
        "INSERT OR REPLACE INTO kv (project_id, key, value, author, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            project_id,
            entry.key().as_ref(),
            entry.value(),
            entry.author().label(),
            *entry.updated_at() as i64
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Deletes the row `sql` names by project and key. Returns whether one went.
fn delete_one(
    conn: &Connection,
    sql: &str,
    project_id: i64,
    key: &str,
) -> Result<bool, ConfigError> {
    let removed = conn
        .execute(sql, params![project_id, key])
        .map_err(db_error)?;
    Ok(removed > 0)
}

impl CoordinationStore for SqliteStore {
    fn scratchpads(&self, project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            collect(
                conn,
                &format!("{SELECT_SCRATCHPAD} WHERE project_id = ?1 ORDER BY rowid"),
                params![project_id],
                scratchpad_from_row,
            )
        })
    }

    fn scratchpad(
        &self,
        project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<Option<Scratchpad>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            Ok(collect(
                conn,
                &format!("{SELECT_SCRATCHPAD} WHERE project_id = ?1 AND key = ?2"),
                params![project_id, key.as_ref()],
                scratchpad_from_row,
            )?
            .pop())
        })
    }

    fn set_scratchpad(
        &self,
        project: &ProjectRoot,
        author: &Author,
        key: &ScratchpadKey,
        body: &str,
    ) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_string())
                .author(author.clone())
                .updated_at(now_epoch_secs() as u64)
                .build();
            put_scratchpad(conn, project_id, &note)
        })
    }

    fn delete_scratchpad(
        &self,
        project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<bool, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            delete_one(
                conn,
                "DELETE FROM scratchpads WHERE project_id = ?1 AND key = ?2",
                project_id,
                key.as_ref(),
            )
        })
    }

    fn todos(&self, project: &ProjectRoot) -> Result<Vec<Todo>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            collect(
                conn,
                &format!("{SELECT_TODO} WHERE project_id = ?1 ORDER BY rowid"),
                params![project_id],
                todo_from_row,
            )
        })
    }

    fn todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<Option<Todo>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            Ok(collect(
                conn,
                &format!("{SELECT_TODO} WHERE project_id = ?1 AND id = ?2"),
                params![project_id, id.as_ref()],
                todo_from_row,
            )?
            .pop())
        })
    }

    fn add_todo(
        &self,
        project: &ProjectRoot,
        author: &Author,
        title: &TodoTitle,
        deps: &[TodoId],
    ) -> Result<Todo, ConfigError> {
        let todo = Todo::builder()
            .id(TodoId::generate()
                .map_err(|error| ConfigError::CoordinationId(error.to_string()))?)
            .title(title.clone())
            .status(TodoStatus::Pending)
            .deps(deps.to_vec())
            .author(author.clone())
            .updated_at(now_epoch_secs() as u64)
            .build();
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            put_todo(conn, project_id, &todo)
        })?;
        Ok(todo)
    }

    fn set_todo_status(
        &self,
        project: &ProjectRoot,
        author: &Author,
        id: &TodoId,
        status: TodoStatus,
    ) -> Result<bool, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            // A claim is an in-progress todo with an owner and a lease, so
            // any other status drops both.
            let changed = conn
                .execute(
                    "UPDATE todos SET status = ?1, author = ?2, updated_at = ?3, \
                     owner = CASE WHEN ?1 = 'in-progress' THEN owner END, \
                     claimed_until = CASE WHEN ?1 = 'in-progress' THEN claimed_until END \
                     WHERE project_id = ?4 AND id = ?5",
                    params![
                        status.to_string(),
                        author.label(),
                        now_epoch_secs(),
                        project_id,
                        id.as_ref()
                    ],
                )
                .map_err(db_error)?;
            Ok(changed > 0)
        })
    }

    fn delete_todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<bool, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            delete_one(
                conn,
                "DELETE FROM todos WHERE project_id = ?1 AND id = ?2",
                project_id,
                id.as_ref(),
            )
        })
    }

    fn values(&self, project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            collect(
                conn,
                &format!("{SELECT_KV} WHERE project_id = ?1 ORDER BY rowid"),
                params![project_id],
                key_value_from_row,
            )
        })
    }

    fn value(&self, project: &ProjectRoot, key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            Ok(collect(
                conn,
                &format!("{SELECT_KV} WHERE project_id = ?1 AND key = ?2"),
                params![project_id, key.as_ref()],
                key_value_from_row,
            )?
            .pop())
        })
    }

    fn set_value(
        &self,
        project: &ProjectRoot,
        author: &Author,
        key: &KvKey,
        value: &str,
    ) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let entry = KeyValue::builder()
                .key(key.clone())
                .value(value.to_string())
                .author(author.clone())
                .updated_at(now_epoch_secs() as u64)
                .build();
            put_value(conn, project_id, &entry)
        })
    }

    fn delete_value(&self, project: &ProjectRoot, key: &KvKey) -> Result<bool, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            delete_one(
                conn,
                "DELETE FROM kv WHERE project_id = ?1 AND key = ?2",
                project_id,
                key.as_ref(),
            )
        })
    }

    fn import(
        &self,
        project: &ProjectRoot,
        scratchpads: &[Scratchpad],
        todos: &[Todo],
        values: &[KeyValue],
    ) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            for note in scratchpads {
                put_scratchpad(conn, project_id, note)?;
            }
            for todo in todos {
                put_todo(conn, project_id, todo)?;
            }
            for entry in values {
                put_value(conn, project_id, entry)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, slice};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{database, fresh_store, spec},
        domain::port::StackStore,
    };

    /// Registers a project at a fresh folder under `scratch`.
    fn project(store: &SqliteStore, scratch: &TempDir, relative: &str) -> ProjectRoot {
        let dir = scratch.path().join(relative);
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        store.add_process(&root, &spec("web", "true")).unwrap();
        root
    }

    fn key(name: &str) -> ScratchpadKey {
        ScratchpadKey::try_new(name).unwrap()
    }

    fn author() -> Author {
        Author::agent("tester")
    }

    /// Writing a scratchpad twice replaces it and stamps the writer.
    #[test]
    fn writes_and_replaces_a_scratchpad() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");

        store
            .set_scratchpad(&root, &author(), &key("plan"), "first")
            .unwrap();
        store
            .set_scratchpad(&root, &author(), &key("plan"), "second")
            .unwrap();

        let all = store.scratchpads(&root).unwrap();
        assert_eq!(all.len(), 1, "the same key is replaced, not duplicated");
        assert_eq!(all[0].body(), "second");
        assert_eq!(all[0].author(), &author());
    }

    /// Entries are keyed by project, so one key holds a value per project.
    #[test]
    fn coordination_is_keyed_by_project() {
        let (store, scratch) = fresh_store();
        let one = project(&store, &scratch, "one");
        let two = project(&store, &scratch, "two");

        store
            .set_scratchpad(&one, &Author::human(), &key("plan"), "one")
            .unwrap();
        store
            .set_scratchpad(&two, &Author::human(), &key("plan"), "two")
            .unwrap();

        assert_eq!(store.scratchpads(&one).unwrap().len(), 1);
        assert_eq!(
            store
                .scratchpad(&two, &key("plan"))
                .unwrap()
                .unwrap()
                .body(),
            "two"
        );
        assert!(store.delete_scratchpad(&one, &key("plan")).unwrap());
        assert!(!store.delete_scratchpad(&one, &key("plan")).unwrap());
        assert!(store.scratchpad(&two, &key("plan")).unwrap().is_some());
    }

    /// Every operation on a folder that is not a project names it.
    #[test]
    fn an_unregistered_project_is_refused() {
        let (store, _scratch) = fresh_store();
        let ghost = ProjectRoot::try_new(PathBuf::from("/nowhere/at/all")).unwrap();

        let refused = |result: Result<(), ConfigError>| matches!(result, Err(ConfigError::UnknownProject { root }) if &root == ghost.as_ref());
        assert!(refused(store.scratchpads(&ghost).map(|_| ())));
        assert!(refused(store.set_scratchpad(
            &ghost,
            &author(),
            &key("k"),
            "v"
        )));
        assert!(refused(store.todos(&ghost).map(|_| ())));
        assert!(refused(
            store
                .add_todo(&ghost, &author(), &TodoTitle::try_new("t").unwrap(), &[])
                .map(|_| ())
        ));
        assert!(refused(store.set_value(
            &ghost,
            &author(),
            &KvKey::try_new("k").unwrap(),
            "v"
        )));
        assert!(refused(CoordinationStore::import(
            &store,
            &ghost,
            &[],
            &[],
            &[]
        )));
    }

    /// A todo round-trips add, status update and delete.
    #[test]
    fn todos_round_trip_add_status_and_delete() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let dep = TodoId::generate().unwrap();

        let created = store
            .add_todo(
                &root,
                &author(),
                &TodoTitle::try_new("wire the store").unwrap(),
                slice::from_ref(&dep),
            )
            .unwrap();
        assert_eq!(*created.status(), TodoStatus::Pending);
        assert_eq!(
            store.todo(&root, created.id()).unwrap().as_ref(),
            Some(&created)
        );

        assert!(
            store
                .set_todo_status(&root, &Author::human(), created.id(), TodoStatus::Done)
                .unwrap()
        );
        let reread = store.todo(&root, created.id()).unwrap().unwrap();
        assert_eq!(*reread.status(), TodoStatus::Done);
        assert_eq!(
            reread.author(),
            &Author::human(),
            "status stamps the writer"
        );
        assert_eq!(reread.deps(), &vec![dep]);

        assert!(store.delete_todo(&root, created.id()).unwrap());
        assert!(!store.delete_todo(&root, created.id()).unwrap());
        assert!(
            !store
                .set_todo_status(&root, &author(), created.id(), TodoStatus::Done)
                .unwrap()
        );
    }

    /// Finishing or reopening a claimed todo drops its owner and lease, so
    /// the schema's claim invariant holds.
    #[test]
    fn a_status_other_than_in_progress_drops_the_claim() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let todo = store
            .add_todo(
                &root,
                &author(),
                &TodoTitle::try_new("claimed").unwrap(),
                &[],
            )
            .unwrap();
        let claim = |conn: &Connection| {
            conn.execute(
                "UPDATE todos SET status = 'in-progress', owner = 'agent', claimed_until = 99 WHERE id = ?1",
                params![todo.id().as_ref()],
            )
            .map_err(db_error)
        };
        let owner = |store: &SqliteStore| -> Option<String> {
            store
                .read(|conn| {
                    conn.query_row(
                        "SELECT owner FROM todos WHERE id = ?1",
                        params![todo.id().as_ref()],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
                })
                .unwrap()
        };
        store.write(claim).unwrap();

        store
            .set_todo_status(&root, &author(), todo.id(), TodoStatus::InProgress)
            .unwrap();
        assert_eq!(owner(&store).as_deref(), Some("agent"), "still in progress");
        store
            .set_todo_status(&root, &author(), todo.id(), TodoStatus::Done)
            .unwrap();
        assert_eq!(owner(&store), None);
    }

    #[test]
    fn key_values_round_trip() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let k = KvKey::try_new("port").unwrap();

        store.set_value(&root, &author(), &k, "3000").unwrap();
        store.set_value(&root, &author(), &k, "4000").unwrap();

        assert_eq!(store.values(&root).unwrap().len(), 1);
        assert_eq!(store.value(&root, &k).unwrap().unwrap().value(), "4000");
        assert!(store.delete_value(&root, &k).unwrap());
        assert!(!store.delete_value(&root, &k).unwrap());
    }

    /// A snapshot restores verbatim: ids, authors and timestamps survive.
    #[test]
    fn import_restores_entries_verbatim() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let note = Scratchpad::builder()
            .key(key("plan"))
            .body("# plan\n- step".to_string())
            .author(Author::human())
            .updated_at(99)
            .build();
        let todo = Todo::builder()
            .id(TodoId::generate().unwrap())
            .title(TodoTitle::try_new("ship").unwrap())
            .status(TodoStatus::Done)
            .deps(vec![TodoId::generate().unwrap()])
            .author(Author::agent("claude"))
            .updated_at(1234)
            .build();
        let entry = KeyValue::builder()
            .key(KvKey::try_new("port").unwrap())
            .value("3000".to_string())
            .author(Author::agent("codex"))
            .updated_at(7)
            .build();

        CoordinationStore::import(
            &store,
            &root,
            slice::from_ref(&note),
            slice::from_ref(&todo),
            slice::from_ref(&entry),
        )
        .unwrap();

        assert_eq!(store.scratchpads(&root).unwrap(), vec![note]);
        assert_eq!(store.todos(&root).unwrap(), vec![todo]);
        assert_eq!(store.values(&root).unwrap(), vec![entry]);
    }

    /// A second connection to the file sees committed writes, as the CLI,
    /// MCP server and service rely on.
    #[test]
    fn a_second_connection_sees_committed_writes() {
        let (writer, scratch) = fresh_store();
        let root = project(&writer, &scratch, "shop");
        writer
            .set_scratchpad(&root, &Author::agent("claude"), &key("progress"), "started")
            .unwrap();

        let reader = SqliteStore::open(&database(&scratch)).unwrap();

        let notes = reader.scratchpads(&root).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].author(), &Author::agent("claude"));
    }
}
