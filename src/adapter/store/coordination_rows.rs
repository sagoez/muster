use rusqlite::{Connection, OptionalExtension, Params, Row, named_params, params};

use super::{
    lock_rows,
    project_rows::require_project,
    sqlite_store::{SqliteStore, corrupt, db_error, epoch_from_secs, epoch_param},
    store_error::StoreError,
};
use crate::{
    adapter::clock::epoch_now,
    domain::{
        config::ConfigError,
        coordination::{
            Author, Claim, ClaimState, KeyValue, KvKey, KvValue, Lock, LockAttempt, LockHolder,
            LockName, LockNote, LockRelease, LockTtl, NewTodoTitle, NoteBody, Scratchpad,
            ScratchpadKey, SessionId, Todo, TodoChange, TodoClaim, TodoId, TodoLease, TodoStatus,
            TodoTitle, TodoUpdate,
        },
        port::CoordinationStore,
        value::{EpochSeconds, ProjectRoot},
    },
};

/// Selects the scratchpad columns [`scratchpad_from_row`] reads by name.
const SELECT_SCRATCHPAD: &str = "SELECT key, body, author, updated_at FROM scratchpads";
/// The columns [`todo_from_row`] reads, in order.
const TODO_COLUMNS: &str = "id, title, status, deps, author, updated_at, owner, claimed_until";
/// Selects the key-value columns [`key_value_from_row`] reads by name.
const SELECT_KV: &str = "SELECT key, value, author, updated_at FROM kv";
/// Claims the todo `:id` for `:me` when it is pending, its lease ran out, or
/// `:me` already holds it; run with `RETURNING` [`TODO_COLUMNS`].
const CLAIM_BY_ID: &str = "\
UPDATE todos SET status = 'in-progress', owner = :me, claimed_until = :until, \
    updated_at = :now, author = :me \
WHERE project_id = :project AND id = :id \
  AND (status = 'pending' \
       OR (status = 'in-progress' AND (owner = :me OR claimed_until < :now)))";
/// The oldest todo of `:project` claimable without naming it: pending or with
/// a lapsed lease, and no dependency that exists and is not done. Mirrors
/// [`claimable`](crate::domain::coordination::claimable).
const NEXT_CLAIMABLE: &str = "\
SELECT t.id FROM todos t \
WHERE t.project_id = :project \
  AND (t.status = 'pending' OR (t.status = 'in-progress' AND t.claimed_until < :now)) \
  AND NOT EXISTS ( \
      SELECT 1 FROM json_each(t.deps) d \
      JOIN todos dep ON dep.project_id = t.project_id AND dep.id = d.value \
      WHERE dep.status <> 'done') \
ORDER BY t.rowid LIMIT 1";

/// A row's stored write time, which the store only ever stamps from the
/// clock.
fn updated_at(row: &Row) -> Result<EpochSeconds, ConfigError> {
    epoch_from_secs("write time", row.get("updated_at").map_err(db_error)?)
}

/// Serializes a todo's dependency ids to the JSON stored in `deps`.
fn deps_json(deps: &[TodoId]) -> Result<String, ConfigError> {
    Ok(serde_json::to_string(deps).map_err(StoreError::Encode)?)
}

/// Reads a [`SELECT_SCRATCHPAD`] row.
fn scratchpad_from_row(row: &Row) -> Result<Scratchpad, ConfigError> {
    let key: String = row.get("key").map_err(db_error)?;
    let body: String = row.get("body").map_err(db_error)?;
    let author: String = row.get("author").map_err(db_error)?;
    Ok(Scratchpad::builder()
        .key(ScratchpadKey::try_new(key).map_err(|error| corrupt("scratchpad key", error))?)
        .body(body)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row)?)
        .build())
}

/// Reads a todo row selected as [`TODO_COLUMNS`].
fn todo_from_row(row: &Row) -> Result<Todo, ConfigError> {
    let id: String = row.get("id").map_err(db_error)?;
    let title: String = row.get("title").map_err(db_error)?;
    let status: String = row.get("status").map_err(db_error)?;
    let deps: String = row.get("deps").map_err(db_error)?;
    let author: String = row.get("author").map_err(db_error)?;
    let owner: Option<String> = row.get("owner").map_err(db_error)?;
    let claimed_until: Option<i64> = row.get("claimed_until").map_err(db_error)?;
    let claim = match owner.zip(claimed_until) {
        Some((owner, until)) => Some(
            TodoClaim::builder()
                .owner(Author::from_label(&owner))
                .claimed_until(epoch_from_secs("todo lease", until)?)
                .build(),
        ),
        None => None,
    };
    Ok(Todo::builder()
        .id(TodoId::try_new(id).map_err(|error| corrupt("todo id", error))?)
        .title(TodoTitle::try_new(title).map_err(|error| corrupt("todo title", error))?)
        .status(
            status
                .parse()
                .map_err(|error| corrupt("todo status", error))?,
        )
        .deps(serde_json::from_str(&deps).map_err(|error| corrupt("todo deps", error))?)
        .claim(claim)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row)?)
        .build())
}

/// Reads a [`SELECT_KV`] row.
fn key_value_from_row(row: &Row) -> Result<KeyValue, ConfigError> {
    let key: String = row.get("key").map_err(db_error)?;
    let value: String = row.get("value").map_err(db_error)?;
    let author: String = row.get("author").map_err(db_error)?;
    Ok(KeyValue::builder()
        .key(KvKey::try_new(key).map_err(|error| corrupt("kv key", error))?)
        .value(value)
        .author(Author::from_label(&author))
        .updated_at(updated_at(row)?)
        .build())
}

/// Every row `sql` selects with `values`, mapped with `map`.
pub(super) fn collect<T>(
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
            epoch_param(*note.updated_at())
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// A todo's claim as its `owner` and `claimed_until` columns.
fn claim_columns(todo: &Todo) -> (Option<&str>, Option<i64>) {
    todo.claim().as_ref().map_or((None, None), |claim| {
        (
            Some(claim.owner().label()),
            Some(epoch_param(*claim.claimed_until())),
        )
    })
}

/// Writes a todo with the author, time and claim it carries.
fn put_todo(conn: &Connection, project_id: i64, todo: &Todo) -> Result<(), ConfigError> {
    let (owner, claimed_until) = claim_columns(todo);
    conn.execute(
        "INSERT OR REPLACE INTO todos \
         (project_id, id, title, status, deps, author, updated_at, owner, claimed_until) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            project_id,
            todo.id().as_ref(),
            todo.title().as_ref(),
            todo.status().to_string(),
            deps_json(todo.deps())?,
            todo.author().label(),
            epoch_param(*todo.updated_at()),
            owner,
            claimed_until
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Rewrites the mutable fields of a stored todo in place, keeping its place
/// in creation order.
fn rewrite_todo(conn: &Connection, project_id: i64, todo: &Todo) -> Result<(), ConfigError> {
    let (owner, claimed_until) = claim_columns(todo);
    conn.execute(
        "UPDATE todos SET title = ?1, status = ?2, author = ?3, updated_at = ?4, owner = ?5, \
         claimed_until = ?6 WHERE project_id = ?7 AND id = ?8",
        params![
            todo.title().as_ref(),
            todo.status().to_string(),
            todo.author().label(),
            epoch_param(*todo.updated_at()),
            owner,
            claimed_until,
            project_id,
            todo.id().as_ref()
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// The todo `id` of project `project_id`, if present.
fn find_todo(conn: &Connection, project_id: i64, id: &TodoId) -> Result<Option<Todo>, ConfigError> {
    Ok(collect(
        conn,
        &format!("SELECT {TODO_COLUMNS} FROM todos WHERE project_id = ?1 AND id = ?2"),
        params![project_id, id.as_ref()],
        todo_from_row,
    )?
    .pop())
}

/// Claims the todo `id` of project `project_id` for `me` until `until`, or
/// says why it cannot be.
fn claim_by_id(
    conn: &Connection,
    project_id: i64,
    me: &Author,
    id: &TodoId,
    until: EpochSeconds,
    now: EpochSeconds,
) -> Result<Claim, ConfigError> {
    let claimed = collect(
        conn,
        &format!("{CLAIM_BY_ID} RETURNING {TODO_COLUMNS}"),
        named_params! {
            ":project": project_id,
            ":id": id.as_ref(),
            ":me": me.label(),
            ":until": epoch_param(until),
            ":now": epoch_param(now),
        },
        todo_from_row,
    )?
    .pop();
    if let Some(todo) = claimed {
        return Ok(Claim::Claimed(todo));
    }
    let Some(todo) = find_todo(conn, project_id, id)? else {
        return Ok(Claim::Missing);
    };
    Ok(match todo.claim_state(now) {
        ClaimState::Held { owner, expires_in } => Claim::HeldBy {
            owner: owner.clone(),
            expires_in,
        },
        ClaimState::Unclaimed | ClaimState::Stale { .. } => Claim::Unavailable(todo),
    })
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
            epoch_param(*entry.updated_at())
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
        body: &NoteBody,
    ) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_string())
                .author(author.clone())
                .updated_at(epoch_now())
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
                &format!("SELECT {TODO_COLUMNS} FROM todos WHERE project_id = ?1 ORDER BY rowid"),
                params![project_id],
                todo_from_row,
            )
        })
    }

    fn todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<Option<Todo>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            find_todo(conn, project_id, id)
        })
    }

    fn add_todo(
        &self,
        project: &ProjectRoot,
        author: &Author,
        title: &NewTodoTitle,
        deps: &[TodoId],
    ) -> Result<Todo, ConfigError> {
        let todo = Todo::builder()
            .id(TodoId::generate()
                .map_err(|error| ConfigError::CoordinationId(error.to_string()))?)
            .title(title.title().clone())
            .status(TodoStatus::Pending)
            .deps(deps.to_vec())
            .claim(None)
            .author(author.clone())
            .updated_at(epoch_now())
            .build();
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            put_todo(conn, project_id, &todo)
        })?;
        Ok(todo)
    }

    fn update_todo(
        &self,
        project: &ProjectRoot,
        me: &Author,
        id: &TodoId,
        change: &TodoChange,
    ) -> Result<TodoUpdate, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let Some(todo) = find_todo(conn, project_id, id)? else {
                return Ok(TodoUpdate::Missing);
            };
            let update = todo.changed(me, change, epoch_now());
            if let TodoUpdate::Updated(changed) = &update {
                rewrite_todo(conn, project_id, changed)?;
            }
            Ok(update)
        })
    }

    fn claim_todo(
        &self,
        project: &ProjectRoot,
        me: &Author,
        id: Option<&TodoId>,
        lease: TodoLease,
    ) -> Result<Claim, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let now = epoch_now();
            let until = now.after(*lease.as_ref());
            if let Some(id) = id {
                return claim_by_id(conn, project_id, me, id, until, now);
            }
            let next: Option<String> = conn
                .query_row(
                    NEXT_CLAIMABLE,
                    named_params! { ":project": project_id, ":now": epoch_param(now) },
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            match next {
                None => Ok(Claim::NothingClaimable),
                Some(next) => {
                    let next = TodoId::try_new(next).map_err(|error| corrupt("todo id", error))?;
                    claim_by_id(conn, project_id, me, &next, until, now)
                },
            }
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
        value: &KvValue,
    ) -> Result<(), ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            let entry = KeyValue::builder()
                .key(key.clone())
                .value(value.to_string())
                .author(author.clone())
                .updated_at(epoch_now())
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

    fn acquire_lock(
        &self,
        project: &ProjectRoot,
        me: &LockHolder,
        name: &LockName,
        ttl: LockTtl,
        note: Option<&LockNote>,
    ) -> Result<LockAttempt, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            lock_rows::acquire(conn, project_id, me, name, ttl, note, epoch_now())
        })
    }

    fn release_lock(
        &self,
        project: &ProjectRoot,
        me: &LockHolder,
        name: &LockName,
    ) -> Result<LockRelease, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            lock_rows::release(conn, project_id, me, name, epoch_now())
        })
    }

    fn break_lock(
        &self,
        project: &ProjectRoot,
        name: &LockName,
    ) -> Result<Option<Lock>, ConfigError> {
        self.write(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            lock_rows::break_lock(conn, project_id, name)
        })
    }

    fn release_locks_of(&self, session: &SessionId) -> Result<usize, ConfigError> {
        self.write(|conn| lock_rows::release_all_of(conn, session))
    }

    fn locks(&self, project: &ProjectRoot) -> Result<Vec<Lock>, ConfigError> {
        self.read(|conn| {
            let (project_id, _) = require_project(conn, project)?;
            lock_rows::live(conn, project_id, epoch_now())
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
    use std::{fs, path::PathBuf, slice, thread, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{database, fresh_store, spec},
        domain::{
            coordination::{TODO_TITLE_MAX_CHARS, claimable},
            port::StackStore,
        },
    };

    /// Registers a project at a fresh folder under `scratch`.
    fn project(store: &SqliteStore, scratch: &TempDir, relative: &str) -> ProjectRoot {
        let dir = scratch.path().join(relative);
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
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
            .set_scratchpad(
                &root,
                &author(),
                &key("plan"),
                &NoteBody::try_new("first").unwrap(),
            )
            .unwrap();
        store
            .set_scratchpad(
                &root,
                &author(),
                &key("plan"),
                &NoteBody::try_new("second").unwrap(),
            )
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
            .set_scratchpad(
                &one,
                &Author::human(),
                &key("plan"),
                &NoteBody::try_new("one").unwrap(),
            )
            .unwrap();
        store
            .set_scratchpad(
                &two,
                &Author::human(),
                &key("plan"),
                &NoteBody::try_new("two").unwrap(),
            )
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
            &NoteBody::try_new("v").unwrap()
        )));
        assert!(refused(store.todos(&ghost).map(|_| ())));
        assert!(refused(
            store
                .add_todo(&ghost, &author(), &NewTodoTitle::try_new("t").unwrap(), &[])
                .map(|_| ())
        ));
        assert!(refused(store.set_value(
            &ghost,
            &author(),
            &KvKey::try_new("k").unwrap(),
            &KvValue::try_new("v").unwrap()
        )));
        assert!(refused(CoordinationStore::import(
            &store,
            &ghost,
            &[],
            &[],
            &[]
        )));
        assert!(refused(
            store
                .claim_todo(&ghost, &author(), None, TodoLease::DEFAULT)
                .map(|_| ())
        ));
        let lock = LockName::try_new("migrations").unwrap();
        assert!(refused(
            store
                .acquire_lock(
                    &ghost,
                    &LockHolder::fresh("tester"),
                    &lock,
                    LockTtl::DEFAULT,
                    None
                )
                .map(|_| ())
        ));
        assert!(refused(store.locks(&ghost).map(|_| ())));
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
                &NewTodoTitle::try_new("wire the store").unwrap(),
                slice::from_ref(&dep),
            )
            .unwrap();
        assert_eq!(*created.status(), TodoStatus::Pending);
        assert_eq!(
            store.todo(&root, created.id()).unwrap().as_ref(),
            Some(&created)
        );

        let TodoUpdate::Updated(updated) = store
            .update_todo(
                &root,
                &Author::human(),
                created.id(),
                &status(TodoStatus::Done, false),
            )
            .unwrap()
        else {
            panic!("an unclaimed todo updates");
        };
        let reread = store.todo(&root, created.id()).unwrap().unwrap();
        assert_eq!(reread, updated, "the result is what was stored");
        assert_eq!(*reread.status(), TodoStatus::Done);
        assert_eq!(
            reread.author(),
            &Author::human(),
            "status stamps the writer"
        );
        assert_eq!(reread.deps(), &vec![dep]);

        assert!(store.delete_todo(&root, created.id()).unwrap());
        assert!(!store.delete_todo(&root, created.id()).unwrap());
        assert_eq!(
            store
                .update_todo(
                    &root,
                    &author(),
                    created.id(),
                    &status(TodoStatus::Done, false)
                )
                .unwrap(),
            TodoUpdate::Missing
        );
    }

    /// A change that sets only `status`, forced or not.
    fn status(status: TodoStatus, force: bool) -> TodoChange {
        TodoChange::builder()
            .title(None)
            .status(Some(status))
            .force(force)
            .build()
    }

    /// A pending todo titled `title` under `root`.
    fn add(store: &SqliteStore, root: &ProjectRoot, title: &str, deps: &[TodoId]) -> Todo {
        store
            .add_todo(
                root,
                &author(),
                &NewTodoTitle::try_new(title).unwrap(),
                deps,
            )
            .unwrap()
    }

    /// Runs the lease of todo `id` out, as if its holder went away an hour ago.
    fn lapse(store: &SqliteStore, id: &TodoId) {
        store
            .write(|conn| {
                conn.execute(
                    "UPDATE todos SET claimed_until = ?1 WHERE id = ?2",
                    params![epoch_param(epoch_now()) - 3600, id.as_ref()],
                )
                .map_err(db_error)
            })
            .unwrap();
    }

    /// A title stored past the write cap, as one written before the cap
    /// existed, still lists: the cap binds writes, never reads.
    #[test]
    fn a_stored_title_past_the_write_cap_still_lists() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let todo = add(&store, &root, "short", &[]);
        let long = "x".repeat(TODO_TITLE_MAX_CHARS + 1);
        store
            .write(|conn| {
                conn.execute("UPDATE todos SET title = ?1 WHERE id = ?2", params![
                    long,
                    todo.id().as_ref()
                ])
                .map_err(db_error)
            })
            .unwrap();

        let listed = store.todos(&root).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title().as_ref(), long);
    }

    /// The todo `claim` took, or a panic naming what it came to.
    fn claimed(claim: Claim) -> Todo {
        match claim {
            Claim::Claimed(todo) => todo,
            other => panic!("expected a claim, got {other:?}"),
        }
    }

    /// Claiming by id takes a pending todo, renews for its owner, answers
    /// another agent with the holder, and lets anyone take a lapsed lease.
    #[test]
    fn claiming_by_id_respects_the_lease() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let task = add(&store, &root, "task", &[]);
        let claude = Author::agent("claude");
        let codex = Author::agent("codex");
        let lease = TodoLease::DEFAULT;

        let held = claimed(
            store
                .claim_todo(&root, &claude, Some(task.id()), lease)
                .unwrap(),
        );
        assert_eq!(*held.status(), TodoStatus::InProgress);
        assert_eq!(held.claim().as_ref().unwrap().owner(), &claude);
        assert_eq!(held.author(), &claude);
        assert_eq!(store.todo(&root, task.id()).unwrap().unwrap(), held);

        let Claim::HeldBy { owner, expires_in } = store
            .claim_todo(&root, &codex, Some(task.id()), lease)
            .unwrap()
        else {
            panic!("another agent's live lease holds");
        };
        assert_eq!(owner, claude);
        assert!(expires_in <= *lease.as_ref() && expires_in > Duration::ZERO);

        lapse(&store, task.id());
        let renewed = claimed(
            store
                .claim_todo(&root, &claude, Some(task.id()), lease)
                .unwrap(),
        );
        assert!(
            renewed.claim().as_ref().unwrap().is_live(epoch_now()),
            "renewal extends it"
        );

        lapse(&store, task.id());
        let taken = claimed(
            store
                .claim_todo(&root, &codex, Some(task.id()), lease)
                .unwrap(),
        );
        assert_eq!(taken.claim().as_ref().unwrap().owner(), &codex);
    }

    /// A done todo, an unowned in-progress one and an unknown id are not
    /// claimable by id.
    #[test]
    fn claiming_by_id_refuses_what_it_cannot_take() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let done = add(&store, &root, "done", &[]);
        let busy = add(&store, &root, "busy", &[]);
        let me = Author::agent("claude");
        let lease = TodoLease::DEFAULT;
        store
            .update_todo(
                &root,
                &Author::human(),
                done.id(),
                &status(TodoStatus::Done, false),
            )
            .unwrap();
        store
            .update_todo(
                &root,
                &Author::human(),
                busy.id(),
                &status(TodoStatus::InProgress, false),
            )
            .unwrap();

        assert!(matches!(
            store.claim_todo(&root, &me, Some(done.id()), lease).unwrap(),
            Claim::Unavailable(found) if found.id() == done.id()
        ));
        assert!(
            matches!(
                store.claim_todo(&root, &me, Some(busy.id()), lease).unwrap(),
                Claim::HeldBy { owner, .. } if owner == Author::human()
            ),
            "setting in-progress claimed it for the human"
        );
        assert_eq!(
            store
                .claim_todo(&root, &me, Some(&TodoId::generate().unwrap()), lease)
                .unwrap(),
            Claim::Missing
        );
    }

    /// Without an id, the oldest todo whose dependencies are done is taken,
    /// lapsed leases included, and the choice agrees with the domain's.
    #[test]
    fn claiming_the_next_todo_follows_deps_and_age() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let first = add(&store, &root, "first", &[]);
        let blocked = add(&store, &root, "blocked", slice::from_ref(first.id()));
        let orphan = add(&store, &root, "orphan", &[TodoId::generate().unwrap()]);
        let claude = Author::agent("claude");
        let codex = Author::agent("codex");
        let lease = TodoLease::DEFAULT;
        let next_by_domain = |store: &SqliteStore| {
            claimable(&store.todos(&root).unwrap(), epoch_now())
                .next()
                .map(|todo| todo.id().clone())
        };

        assert_eq!(next_by_domain(&store).as_ref(), Some(first.id()));
        let one = claimed(store.claim_todo(&root, &claude, None, lease).unwrap());
        assert_eq!(one.id(), first.id());

        assert_eq!(next_by_domain(&store).as_ref(), Some(orphan.id()));
        let two = claimed(store.claim_todo(&root, &codex, None, lease).unwrap());
        assert_eq!(
            two.id(),
            orphan.id(),
            "a vanished dependency does not block"
        );

        assert_eq!(next_by_domain(&store), None);
        assert_eq!(
            store.claim_todo(&root, &codex, None, lease).unwrap(),
            Claim::NothingClaimable,
            "the blocked one waits on an unfinished dependency"
        );

        lapse(&store, first.id());
        assert_eq!(next_by_domain(&store).as_ref(), Some(first.id()));
        let three = claimed(store.claim_todo(&root, &codex, None, lease).unwrap());
        assert_eq!(three.id(), first.id(), "a lapsed lease is claimable again");

        store
            .update_todo(&root, &codex, first.id(), &status(TodoStatus::Done, false))
            .unwrap();
        let four = claimed(store.claim_todo(&root, &claude, None, lease).unwrap());
        assert_eq!(four.id(), blocked.id());
    }

    /// Two stores on one file racing for the next todo never both win it.
    #[test]
    fn concurrent_claims_take_distinct_todos() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        add(&store, &root, "only", &[]);
        let racers: Vec<_> = (0..4)
            .map(|index| {
                let store = SqliteStore::open(&database(&scratch)).unwrap();
                let root = root.clone();
                thread::spawn(move || {
                    store
                        .claim_todo(
                            &root,
                            &Author::agent(&format!("agent-{index}")),
                            None,
                            TodoLease::DEFAULT,
                        )
                        .unwrap()
                })
            })
            .collect();

        let wins = racers
            .into_iter()
            .map(|racer| racer.join().unwrap())
            .filter(|claim| matches!(claim, Claim::Claimed(_)))
            .count();

        assert_eq!(wins, 1);
    }

    /// A held todo refuses another agent's change unless forced; finishing or
    /// releasing drops the claim, so the schema's claim invariant holds.
    #[test]
    fn updating_respects_the_lease_and_drops_the_claim() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let task = add(&store, &root, "task", &[]);
        let claude = Author::agent("claude");
        store
            .claim_todo(&root, &claude, Some(task.id()), TodoLease::DEFAULT)
            .unwrap();

        assert!(matches!(
            store
                .update_todo(&root, &Author::human(), task.id(), &status(TodoStatus::Done, false))
                .unwrap(),
            TodoUpdate::HeldBy { owner, .. } if owner == claude
        ));
        let TodoUpdate::Updated(released) = store
            .update_todo(
                &root,
                &claude,
                task.id(),
                &status(TodoStatus::Pending, false),
            )
            .unwrap()
        else {
            panic!("the owner releases");
        };
        assert_eq!(released.claim(), &None);

        store
            .claim_todo(&root, &claude, Some(task.id()), TodoLease::DEFAULT)
            .unwrap();
        let TodoUpdate::Updated(forced) = store
            .update_todo(
                &root,
                &Author::human(),
                task.id(),
                &status(TodoStatus::Done, true),
            )
            .unwrap()
        else {
            panic!("force overrides the lease");
        };
        assert_eq!(*forced.status(), TodoStatus::Done);
        assert_eq!(
            store.todo(&root, task.id()).unwrap().unwrap().claim(),
            &None
        );
    }

    /// Taking a lock is exclusive until released or expired; the holder
    /// renews it; another caller hears who holds it and why.
    #[test]
    fn a_lock_is_exclusive_until_released_or_expired() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let name = LockName::try_new("migrations").unwrap();
        let note = LockNote::try_new("running 0042").unwrap();
        let claude = LockHolder::fresh("claude");
        let codex = LockHolder::fresh("codex");
        let ttl = LockTtl::DEFAULT;

        assert_eq!(
            store
                .acquire_lock(&root, &claude, &name, ttl, Some(&note))
                .unwrap(),
            LockAttempt::Acquired {
                expires_in: *ttl.as_ref()
            }
        );
        let LockAttempt::HeldBy {
            owner,
            note: held_note,
            expires_in,
        } = store.acquire_lock(&root, &codex, &name, ttl, None).unwrap()
        else {
            panic!("the lock is held");
        };
        assert_eq!((owner, held_note), (claude.author().clone(), Some(note)));
        assert!(expires_in <= *ttl.as_ref());
        assert!(matches!(
            store
                .acquire_lock(&root, &claude, &name, ttl, None)
                .unwrap(),
            LockAttempt::Acquired { .. }
        ));
        assert_eq!(
            store.locks(&root).unwrap()[0].note(),
            &None,
            "renewal replaces the note"
        );

        assert_eq!(
            store.release_lock(&root, &codex, &name).unwrap(),
            LockRelease::HeldBy {
                owner: claude.author().clone()
            }
        );
        assert_eq!(
            store.release_lock(&root, &claude, &name).unwrap(),
            LockRelease::Released
        );
        assert_eq!(
            store.release_lock(&root, &claude, &name).unwrap(),
            LockRelease::NotHeld
        );

        store
            .acquire_lock(&root, &claude, &name, ttl, None)
            .unwrap();
        expire_locks(&store);
        assert!(
            store.locks(&root).unwrap().is_empty(),
            "an expired lock is not listed"
        );
        assert_eq!(
            store.release_lock(&root, &codex, &name).unwrap(),
            LockRelease::NotHeld
        );
        assert!(matches!(
            store.acquire_lock(&root, &codex, &name, ttl, None).unwrap(),
            LockAttempt::Acquired { .. }
        ));
        assert_eq!(store.locks(&root).unwrap()[0].owner(), codex.author());
    }

    /// Two connections under one label are two holders: neither renews,
    /// releases nor, on disconnect, drops the other's lock.
    #[test]
    fn a_lock_belongs_to_its_session_not_its_label() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let name = LockName::try_new("migrations").unwrap();
        let first = LockHolder::fresh("claude");
        let second = LockHolder::fresh("claude");
        let ttl = LockTtl::DEFAULT;

        store.acquire_lock(&root, &first, &name, ttl, None).unwrap();
        assert!(matches!(
            store
                .acquire_lock(&root, &second, &name, ttl, None)
                .unwrap(),
            LockAttempt::HeldBy { .. }
        ));
        assert!(matches!(
            store.release_lock(&root, &second, &name).unwrap(),
            LockRelease::HeldBy { .. }
        ));
        assert_eq!(store.release_locks_of(second.session()).unwrap(), 0);
        assert_eq!(store.locks(&root).unwrap().len(), 1);

        assert_eq!(store.release_locks_of(first.session()).unwrap(), 1);
        assert!(matches!(
            store
                .acquire_lock(&root, &second, &name, ttl, None)
                .unwrap(),
            LockAttempt::Acquired { .. }
        ));
        assert_eq!(store.release_locks_of(first.session()).unwrap(), 0);
        assert_eq!(store.locks(&root).unwrap().len(), 1);
    }

    /// Runs every lock out, as if their holders went away an hour ago.
    fn expire_locks(store: &SqliteStore) {
        store
            .write(|conn| {
                conn.execute("UPDATE locks SET expires_at = ?1", params![
                    epoch_param(epoch_now()) - 3600
                ])
                .map_err(db_error)
            })
            .unwrap();
    }

    /// Locks are per project; an exiting agent releases its own everywhere;
    /// the human breaks anyone's.
    #[test]
    fn locks_are_per_project_and_released_by_owner() {
        let (store, scratch) = fresh_store();
        let one = project(&store, &scratch, "one");
        let two = project(&store, &scratch, "two");
        let name = LockName::try_new("db-reset").unwrap();
        let other = LockName::try_new("deploy").unwrap();
        let claude = LockHolder::fresh("claude");
        let codex = LockHolder::fresh("codex");
        let ttl = LockTtl::DEFAULT;

        store.acquire_lock(&one, &claude, &name, ttl, None).unwrap();
        assert!(matches!(
            store.acquire_lock(&two, &codex, &name, ttl, None).unwrap(),
            LockAttempt::Acquired { .. }
        ));
        store
            .acquire_lock(&two, &claude, &other, ttl, None)
            .unwrap();

        assert_eq!(store.release_locks_of(claude.session()).unwrap(), 2);
        assert!(store.locks(&one).unwrap().is_empty());
        let left: Vec<_> = store
            .locks(&two)
            .unwrap()
            .into_iter()
            .map(|lock| lock.owner().clone())
            .collect();
        assert_eq!(left, vec![codex.author().clone()]);

        let broken = store.break_lock(&two, &name).unwrap().unwrap();
        assert_eq!(broken.owner(), codex.author());
        assert_eq!(store.break_lock(&two, &name).unwrap(), None);
    }

    #[test]
    fn key_values_round_trip() {
        let (store, scratch) = fresh_store();
        let root = project(&store, &scratch, "shop");
        let k = KvKey::try_new("port").unwrap();

        store
            .set_value(&root, &author(), &k, &KvValue::try_new("3000").unwrap())
            .unwrap();
        store
            .set_value(&root, &author(), &k, &KvValue::try_new("4000").unwrap())
            .unwrap();

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
            .updated_at(EpochSeconds::new(99))
            .build();
        let todo = Todo::builder()
            .id(TodoId::generate().unwrap())
            .title(TodoTitle::try_new("ship").unwrap())
            .status(TodoStatus::Done)
            .deps(vec![TodoId::generate().unwrap()])
            .claim(None)
            .author(Author::agent("claude"))
            .updated_at(EpochSeconds::new(1234))
            .build();
        let held = Todo::builder()
            .id(TodoId::generate().unwrap())
            .title(TodoTitle::try_new("review").unwrap())
            .status(TodoStatus::InProgress)
            .deps(vec![])
            .claim(Some(
                TodoClaim::builder()
                    .owner(Author::agent("codex"))
                    .claimed_until(EpochSeconds::new(5678))
                    .build(),
            ))
            .author(Author::agent("codex"))
            .updated_at(EpochSeconds::new(1200))
            .build();
        let entry = KeyValue::builder()
            .key(KvKey::try_new("port").unwrap())
            .value("3000".to_string())
            .author(Author::agent("codex"))
            .updated_at(EpochSeconds::new(7))
            .build();

        CoordinationStore::import(
            &store,
            &root,
            slice::from_ref(&note),
            &[todo.clone(), held.clone()],
            slice::from_ref(&entry),
        )
        .unwrap();

        assert_eq!(store.scratchpads(&root).unwrap(), vec![note]);
        assert_eq!(store.todos(&root).unwrap(), vec![todo, held]);
        assert_eq!(store.values(&root).unwrap(), vec![entry]);
    }

    /// A second connection to the file sees committed writes, as the CLI,
    /// MCP server and service rely on.
    #[test]
    fn a_second_connection_sees_committed_writes() {
        let (writer, scratch) = fresh_store();
        let root = project(&writer, &scratch, "shop");
        writer
            .set_scratchpad(
                &root,
                &Author::agent("claude"),
                &key("progress"),
                &NoteBody::try_new("started").unwrap(),
            )
            .unwrap();

        let reader = SqliteStore::open(&database(&scratch)).unwrap();

        let notes = reader.scratchpads(&root).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].author(), &Author::agent("claude"));
    }
}
