use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, Row, params};

use super::yaml::state_dir_path;
use crate::domain::{
    config::ConfigError,
    coordination::{
        Author, KeyValue, KvKey, Scratchpad, ScratchpadKey, Todo, TodoId, TodoStatus, TodoTitle,
    },
    port::CoordinationStore,
};

/// Coordination database filename under muster's platform state directory.
const COORDINATION_DB_FILE: &str = "coordination.db";

/// Schema and connection pragmas, applied on every open. WAL lets the CLI, the
/// MCP subprocess, and the TUI read concurrently with a single writer; the busy
/// timeout bounds the wait for that writer instead of failing fast. Tables are
/// keyed by `(project, key/id)`, so a write touches one row, not the whole store.
const SCHEMA: &str = "\
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS scratchpads (
    project    TEXT NOT NULL,
    key        TEXT NOT NULL,
    body       TEXT NOT NULL,
    author     TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project, key)
);
CREATE TABLE IF NOT EXISTS todos (
    project    TEXT NOT NULL,
    id         TEXT NOT NULL,
    title      TEXT NOT NULL,
    status     TEXT NOT NULL,
    deps       TEXT NOT NULL,
    author     TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project, id)
);
CREATE TABLE IF NOT EXISTS kv (
    project    TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    author     TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project, key)
);";

/// Seconds since the Unix epoch, or zero if the clock predates it. The store
/// stamps write times here so the pure domain need not depend on a clock.
fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Maps a database driver error to the domain error, keeping `rusqlite` out of
/// the domain.
fn db_error(error: rusqlite::Error) -> ConfigError {
    ConfigError::Database(error.to_string())
}

/// The stable text key for a project (its absolutized `muster.yml` path).
fn project_key(project: &Path) -> String {
    project.to_string_lossy().into_owned()
}

/// Serializes a todo's dependency ids to the JSON stored in the `deps` column.
fn deps_json(deps: &[TodoId]) -> Result<String, ConfigError> {
    serde_json::to_string(deps).map_err(|error| ConfigError::Database(error.to_string()))
}

/// SQLite-backed coordination store shared by the CLI and the MCP server. Each
/// write touches one row under SQLite's own cross-process locking (WAL); there is
/// no whole-file rewrite and no hand-rolled lock. The writer's identity is passed
/// per call, so one process can persist writes from the human and from an agent.
pub struct SqliteCoordinationStore {
    conn: Mutex<Connection>,
}

impl SqliteCoordinationStore {
    /// Opens the store at muster's platform state path, creating it if absent.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the state directory is unavailable or the
    /// database cannot be opened or initialized.
    pub fn open_default() -> Result<Self, ConfigError> {
        let path = state_dir_path(COORDINATION_DB_FILE).ok_or(ConfigError::NoConfigDir)?;
        Self::open(&path)
    }

    /// Opens (creating if needed) the coordination database at `path` and applies
    /// the schema and connection pragmas.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the parent directory or database cannot be
    /// created, opened, or initialized.
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path).map_err(db_error)?;
        conn.execute_batch(SCHEMA).map_err(db_error)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Locks the connection for one operation.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if a previous holder panicked and poisoned it.
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ConfigError> {
        self.conn
            .lock()
            .map_err(|_| ConfigError::Database("coordination database lock poisoned".to_string()))
    }
}

/// Reads a scratchpad row (`key, body, author, updated_at`) into an entity.
fn scratchpad_from_row(row: &Row) -> Result<Scratchpad, ConfigError> {
    let key: String = row.get(0).map_err(db_error)?;
    let body: String = row.get(1).map_err(db_error)?;
    let author: String = row.get(2).map_err(db_error)?;
    let updated_at: i64 = row.get(3).map_err(db_error)?;
    Ok(Scratchpad::builder()
        .key(
            ScratchpadKey::try_new(key).map_err(|error| {
                ConfigError::Database(format!("stored scratchpad key: {error}"))
            })?,
        )
        .body(body)
        .author(Author::from_label(&author))
        .updated_at(updated_at as u64)
        .build())
}

/// Reads a todo row (`id, title, status, deps, author, updated_at`) into an entity.
fn todo_from_row(row: &Row) -> Result<Todo, ConfigError> {
    let id: String = row.get(0).map_err(db_error)?;
    let title: String = row.get(1).map_err(db_error)?;
    let status: String = row.get(2).map_err(db_error)?;
    let deps: String = row.get(3).map_err(db_error)?;
    let author: String = row.get(4).map_err(db_error)?;
    let updated_at: i64 = row.get(5).map_err(db_error)?;
    Ok(Todo::builder()
        .id(TodoId::try_new(id)
            .map_err(|error| ConfigError::Database(format!("stored todo id: {error}")))?)
        .title(
            TodoTitle::try_new(title)
                .map_err(|error| ConfigError::Database(format!("stored todo title: {error}")))?,
        )
        .status(
            status
                .parse::<TodoStatus>()
                .map_err(|error| ConfigError::Database(format!("stored todo status: {error}")))?,
        )
        .deps(
            serde_json::from_str(&deps)
                .map_err(|error| ConfigError::Database(format!("stored todo deps: {error}")))?,
        )
        .author(Author::from_label(&author))
        .updated_at(updated_at as u64)
        .build())
}

/// Reads a key-value row (`key, value, author, updated_at`) into an entity.
fn key_value_from_row(row: &Row) -> Result<KeyValue, ConfigError> {
    let key: String = row.get(0).map_err(db_error)?;
    let value: String = row.get(1).map_err(db_error)?;
    let author: String = row.get(2).map_err(db_error)?;
    let updated_at: i64 = row.get(3).map_err(db_error)?;
    Ok(KeyValue::builder()
        .key(
            KvKey::try_new(key)
                .map_err(|error| ConfigError::Database(format!("stored kv key: {error}")))?,
        )
        .value(value)
        .author(Author::from_label(&author))
        .updated_at(updated_at as u64)
        .build())
}

/// Collects every row a prepared query returns, mapping each with `map`.
fn collect<T>(
    conn: &Connection,
    sql: &str,
    project: &Path,
    map: impl Fn(&Row) -> Result<T, ConfigError>,
) -> Result<Vec<T>, ConfigError> {
    let mut statement = conn.prepare(sql).map_err(db_error)?;
    let mut rows = statement
        .query(params![project_key(project)])
        .map_err(db_error)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        out.push(map(row)?);
    }
    Ok(out)
}

impl CoordinationStore for SqliteCoordinationStore {
    fn scratchpads(&self, project: &Path) -> Result<Vec<Scratchpad>, ConfigError> {
        let conn = self.lock()?;
        collect(
            &conn,
            "SELECT key, body, author, updated_at FROM scratchpads WHERE project = ?1 ORDER BY rowid",
            project,
            scratchpad_from_row,
        )
    }

    fn scratchpad(
        &self,
        project: &Path,
        key: &ScratchpadKey,
    ) -> Result<Option<Scratchpad>, ConfigError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT key, body, author, updated_at FROM scratchpads WHERE project = ?1 AND key = ?2",
            )
            .map_err(db_error)?;
        let mut rows = statement
            .query(params![project_key(project), key.as_ref()])
            .map_err(db_error)?;
        match rows.next().map_err(db_error)? {
            Some(row) => Ok(Some(scratchpad_from_row(row)?)),
            None => Ok(None),
        }
    }

    fn set_scratchpad(
        &self,
        project: &Path,
        author: &Author,
        key: &ScratchpadKey,
        body: &str,
    ) -> Result<(), ConfigError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO scratchpads (project, key, body, author, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project_key(project),
                key.as_ref(),
                body,
                author.label(),
                now_epoch_secs() as i64
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }

    fn delete_scratchpad(&self, project: &Path, key: &ScratchpadKey) -> Result<bool, ConfigError> {
        let conn = self.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM scratchpads WHERE project = ?1 AND key = ?2",
                params![project_key(project), key.as_ref()],
            )
            .map_err(db_error)?;
        Ok(removed > 0)
    }

    fn todos(&self, project: &Path) -> Result<Vec<Todo>, ConfigError> {
        let conn = self.lock()?;
        collect(
            &conn,
            "SELECT id, title, status, deps, author, updated_at FROM todos WHERE project = ?1 ORDER BY rowid",
            project,
            todo_from_row,
        )
    }

    fn todo(&self, project: &Path, id: &TodoId) -> Result<Option<Todo>, ConfigError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT id, title, status, deps, author, updated_at FROM todos WHERE project = ?1 AND id = ?2",
            )
            .map_err(db_error)?;
        let mut rows = statement
            .query(params![project_key(project), id.as_ref()])
            .map_err(db_error)?;
        match rows.next().map_err(db_error)? {
            Some(row) => Ok(Some(todo_from_row(row)?)),
            None => Ok(None),
        }
    }

    fn add_todo(
        &self,
        project: &Path,
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
            .updated_at(now_epoch_secs())
            .build();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO todos (project, id, title, status, deps, author, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                project_key(project),
                todo.id().as_ref(),
                todo.title().as_ref(),
                todo.status().to_string(),
                deps_json(deps)?,
                author.label(),
                *todo.updated_at() as i64
            ],
        )
        .map_err(db_error)?;
        Ok(todo)
    }

    fn set_todo_status(
        &self,
        project: &Path,
        author: &Author,
        id: &TodoId,
        status: TodoStatus,
    ) -> Result<bool, ConfigError> {
        let conn = self.lock()?;
        let changed = conn
            .execute(
                "UPDATE todos SET status = ?1, author = ?2, updated_at = ?3 \
                 WHERE project = ?4 AND id = ?5",
                params![
                    status.to_string(),
                    author.label(),
                    now_epoch_secs() as i64,
                    project_key(project),
                    id.as_ref()
                ],
            )
            .map_err(db_error)?;
        Ok(changed > 0)
    }

    fn delete_todo(&self, project: &Path, id: &TodoId) -> Result<bool, ConfigError> {
        let conn = self.lock()?;
        let removed = conn
            .execute("DELETE FROM todos WHERE project = ?1 AND id = ?2", params![
                project_key(project),
                id.as_ref()
            ])
            .map_err(db_error)?;
        Ok(removed > 0)
    }

    fn values(&self, project: &Path) -> Result<Vec<KeyValue>, ConfigError> {
        let conn = self.lock()?;
        collect(
            &conn,
            "SELECT key, value, author, updated_at FROM kv WHERE project = ?1 ORDER BY rowid",
            project,
            key_value_from_row,
        )
    }

    fn value(&self, project: &Path, key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT key, value, author, updated_at FROM kv WHERE project = ?1 AND key = ?2",
            )
            .map_err(db_error)?;
        let mut rows = statement
            .query(params![project_key(project), key.as_ref()])
            .map_err(db_error)?;
        match rows.next().map_err(db_error)? {
            Some(row) => Ok(Some(key_value_from_row(row)?)),
            None => Ok(None),
        }
    }

    fn set_value(
        &self,
        project: &Path,
        author: &Author,
        key: &KvKey,
        value: &str,
    ) -> Result<(), ConfigError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO kv (project, key, value, author, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project_key(project),
                key.as_ref(),
                value,
                author.label(),
                now_epoch_secs() as i64
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }

    fn delete_value(&self, project: &Path, key: &KvKey) -> Result<bool, ConfigError> {
        let conn = self.lock()?;
        let removed = conn
            .execute("DELETE FROM kv WHERE project = ?1 AND key = ?2", params![
                project_key(project),
                key.as_ref()
            ])
            .map_err(db_error)?;
        Ok(removed > 0)
    }

    fn import(
        &self,
        project: &Path,
        scratchpads: &[Scratchpad],
        todos: &[Todo],
        values: &[KeyValue],
    ) -> Result<(), ConfigError> {
        let mut conn = self.lock()?;
        // One transaction: a snapshot restores whole or not at all.
        let transaction = conn.transaction().map_err(db_error)?;
        let project = project_key(project);
        for note in scratchpads {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO scratchpads (project, key, body, author, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        project,
                        note.key().as_ref(),
                        note.body(),
                        note.author().label(),
                        *note.updated_at() as i64
                    ],
                )
                .map_err(db_error)?;
        }
        for todo in todos {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO todos \
                     (project, id, title, status, deps, author, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        project,
                        todo.id().as_ref(),
                        todo.title().as_ref(),
                        todo.status().to_string(),
                        deps_json(todo.deps())?,
                        todo.author().label(),
                        *todo.updated_at() as i64
                    ],
                )
                .map_err(db_error)?;
        }
        for entry in values {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO kv (project, key, value, author, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        project,
                        entry.key().as_ref(),
                        entry.value(),
                        entry.author().label(),
                        *entry.updated_at() as i64
                    ],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A store over a fresh temp-file database, plus the directory to clean up.
    fn store() -> (SqliteCoordinationStore, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("muster-coordination-{}", uuid::Uuid::new_v4()));
        let store = SqliteCoordinationStore::open(&dir.join(COORDINATION_DB_FILE)).unwrap();
        (store, dir)
    }

    fn key(name: &str) -> ScratchpadKey {
        ScratchpadKey::try_new(name).unwrap()
    }

    fn author() -> Author {
        Author::agent("tester")
    }

    /// Writing a scratchpad then reading it back returns the same body and
    /// author; a second write to the same key replaces rather than duplicates.
    #[test]
    fn writes_and_replaces_a_scratchpad() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");

        store
            .set_scratchpad(&project, &author(), &key("plan"), "first")
            .unwrap();
        store
            .set_scratchpad(&project, &author(), &key("plan"), "second")
            .unwrap();

        let all = store.scratchpads(&project).unwrap();
        assert_eq!(all.len(), 1, "the same key is replaced, not duplicated");
        assert_eq!(all[0].body(), "second");
        assert_eq!(all[0].author(), &author(), "the writer is stamped");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Scratchpads are isolated per project.
    #[test]
    fn scratchpads_are_scoped_to_their_project() {
        let (store, dir) = store();
        let one = PathBuf::from("/repo/one/muster.yml");
        let two = PathBuf::from("/repo/two/muster.yml");

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
        fs::remove_dir_all(dir).unwrap();
    }

    /// Deleting reports whether a note went and leaves other keys intact.
    #[test]
    fn deletes_only_the_named_scratchpad() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        store
            .set_scratchpad(&project, &author(), &key("plan"), "body")
            .unwrap();
        store
            .set_scratchpad(&project, &author(), &key("notes"), "body")
            .unwrap();

        assert!(store.delete_scratchpad(&project, &key("plan")).unwrap());
        assert!(!store.delete_scratchpad(&project, &key("plan")).unwrap());
        let remaining = store.scratchpads(&project).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key(), &key("notes"));
        fs::remove_dir_all(dir).unwrap();
    }

    /// A todo round-trips add, status update (preserving order + deps), delete.
    #[test]
    fn todos_round_trip_add_status_and_delete() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        let title = TodoTitle::try_new("wire the store").unwrap();
        let dep = TodoId::generate().unwrap();

        let created = store
            .add_todo(&project, &author(), &title, std::slice::from_ref(&dep))
            .unwrap();
        assert_eq!(*created.status(), TodoStatus::Pending);
        assert_eq!(created.deps(), &vec![dep]);

        assert!(
            store
                .set_todo_status(&project, &Author::human(), created.id(), TodoStatus::Done)
                .unwrap()
        );
        let reread = store.todo(&project, created.id()).unwrap().unwrap();
        assert_eq!(*reread.status(), TodoStatus::Done);
        assert_eq!(
            reread.author(),
            &Author::human(),
            "status stamps the new writer"
        );

        assert!(store.delete_todo(&project, created.id()).unwrap());
        assert!(store.todos(&project).unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Status update / delete against an unknown id reports not-found.
    #[test]
    fn todo_operations_on_an_unknown_id_report_absent() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        let ghost = TodoId::generate().unwrap();
        assert!(
            !store
                .set_todo_status(&project, &author(), &ghost, TodoStatus::Done)
                .unwrap()
        );
        assert!(!store.delete_todo(&project, &ghost).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Key-values replace in place, read back per key, and delete by key.
    #[test]
    fn key_values_round_trip() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        let k = KvKey::try_new("port").unwrap();

        store.set_value(&project, &author(), &k, "3000").unwrap();
        store.set_value(&project, &author(), &k, "4000").unwrap();
        let all = store.values(&project).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].value(), "4000");
        assert!(store.delete_value(&project, &k).unwrap());
        assert!(!store.delete_value(&project, &k).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    /// The three primitives coexist in one database without colliding, and a
    /// multiline markdown body round-trips exactly (the case YAML made awkward).
    #[test]
    fn the_three_primitives_and_a_markdown_body_coexist() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        let body = "# Plan\n\n- step one:  do it\n- step two: \"quote\" it\n";

        store
            .set_scratchpad(&project, &author(), &key("plan"), body)
            .unwrap();
        store
            .add_todo(
                &project,
                &author(),
                &TodoTitle::try_new("task").unwrap(),
                &[],
            )
            .unwrap();
        store
            .set_value(&project, &author(), &KvKey::try_new("k").unwrap(), "v")
            .unwrap();

        assert_eq!(
            store
                .scratchpad(&project, &key("plan"))
                .unwrap()
                .unwrap()
                .body(),
            body
        );
        assert_eq!(store.todos(&project).unwrap().len(), 1);
        assert_eq!(store.values(&project).unwrap().len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    /// A snapshot restores verbatim: ids, authors, and timestamps survive rather
    /// than being re-stamped, so todo dependencies keep pointing at real ids.
    #[test]
    fn import_restores_entries_verbatim() {
        let (store, dir) = store();
        let project = PathBuf::from("/repo/muster.yml");
        let dep = TodoId::generate().unwrap();
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
            .deps(vec![dep.clone()])
            .author(Author::agent("claude"))
            .updated_at(1234)
            .build();
        let entry = KeyValue::builder()
            .key(KvKey::try_new("port").unwrap())
            .value("3000".to_string())
            .author(Author::agent("codex"))
            .updated_at(7)
            .build();

        store
            .import(
                &project,
                std::slice::from_ref(&note),
                std::slice::from_ref(&todo),
                std::slice::from_ref(&entry),
            )
            .unwrap();

        assert_eq!(store.scratchpads(&project).unwrap(), vec![note]);
        let restored = store.todos(&project).unwrap();
        assert_eq!(restored, vec![todo]);
        assert_eq!(restored[0].deps(), &vec![dep], "dependency ids survive");
        assert_eq!(store.values(&project).unwrap(), vec![entry]);
        fs::remove_dir_all(dir).unwrap();
    }

    /// A second connection to the same database file sees another's committed
    /// writes - the cross-process sharing the CLI, MCP, and TUI rely on.
    #[test]
    fn a_second_connection_sees_committed_writes() {
        let dir =
            std::env::temp_dir().join(format!("muster-coordination-{}", uuid::Uuid::new_v4()));
        let path = dir.join(COORDINATION_DB_FILE);
        let project = PathBuf::from("/repo/muster.yml");

        let writer = SqliteCoordinationStore::open(&path).unwrap();
        writer
            .set_scratchpad(
                &project,
                &Author::agent("claude"),
                &key("progress"),
                "started",
            )
            .unwrap();

        let reader = SqliteCoordinationStore::open(&path).unwrap();
        let notes = reader.scratchpads(&project).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body(), "started");
        assert_eq!(notes[0].author(), &Author::agent("claude"));
        fs::remove_dir_all(dir).unwrap();
    }
}
