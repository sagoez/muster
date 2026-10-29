use serde_json::json;

use super::{
    answer::Answer, error::CliError, kv_command::KvCommand, report::attribution, row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::wire::WireForm,
    domain::{
        coordination::{Author, KvKey},
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// Line printed when the project has no key-value entries yet.
const EMPTY_NOTE: &str = "no values; write one with `muster kv set <key> <value>`";
/// Columns between a listed entry's key and value.
const FIELD_GAP: &str = "  ";
/// The JSON key of the listed entries.
const ENTRIES: &str = "entries";
/// The JSON key of the key a write saved.
const SAVED: &str = "saved";
/// The JSON key of the key a delete named.
const KEY: &str = "key";
/// The JSON key of whether a delete found something.
const DELETED: &str = "deleted";

/// Runs a `muster kv` action against `store` for `project`, returning what to
/// print. Absent a subcommand, lists the project's entries.
///
/// # Errors
/// Returns [`CliError`] when a key is invalid, a requested value is absent, or the
/// store cannot be read or written.
pub fn kv(
    command: Option<KvCommand>,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
) -> Result<Answer, CliError> {
    match command {
        None => list(store, project),
        Some(KvCommand::Get { key }) => get(store, project, &key),
        Some(KvCommand::Set { key, value }) => set(store, project, author, &key, &value),
        Some(KvCommand::Rm { key }) => remove(store, project, &key),
    }
}

/// Parses a user-supplied key into a validated [`KvKey`].
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] when the key is blank or whitespace-only.
fn parse_key(key: &str) -> Result<KvKey, CliError> {
    KvKey::try_new(key).map_err(|_| CliError::InvalidKvKey(key.to_string()))
}

/// Lists every entry for `project` as `key  value`, one per line.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(store: &dyn CoordinationStore, project: &ProjectRoot) -> Result<Answer, CliError> {
    let entries = store.values(project)?;
    let wire: Vec<_> = entries.iter().map(WireForm::to_wire).collect();
    let rows = if entries.is_empty() {
        vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]
    } else {
        entries
            .iter()
            .map(|entry| {
                Row::unlabeled(
                    RowKind::Plain,
                    format!(
                        "{key}{FIELD_GAP}{value}{attribution}",
                        key = entry.key().as_ref(),
                        value = entry.value(),
                        attribution = attribution(entry.author()),
                    ),
                )
            })
            .collect()
    };
    Answer::of(rows, &json!({ ENTRIES: wire }))
}

/// Prints the value under `key`, one row per line so the framed layout stays
/// intact and piped output reproduces the value verbatim.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, [`CliError::UnknownValue`]
/// when no value is stored under it, or a store read failure.
fn get(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let entry = store
        .value(project, &parsed)?
        .ok_or_else(|| CliError::UnknownValue(key.to_string()))?;
    let rows = entry
        .value()
        .lines()
        .map(|line| Row::unlabeled(RowKind::Plain, line.to_string()))
        .collect();
    Answer::of(rows, &entry.to_wire())
}

/// Creates or replaces the entry under `key` with `value`.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, or a store write failure.
fn set(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
    key: &str,
    value: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    store.set_value(project, author, &parsed, value)?;
    Answer::of(
        vec![Row::unlabeled(
            RowKind::Ok,
            format!("saved '{}'", parsed.as_ref()),
        )],
        &json!({ SAVED: parsed.as_ref() }),
    )
}

/// Deletes the entry under `key`. Deleting an absent entry is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, or a store write failure.
fn remove(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let deleted = store.delete_value(project, &parsed)?;
    let row = if deleted {
        Row::unlabeled(RowKind::Ok, format!("removed '{}'", parsed.as_ref()))
    } else {
        Row::unlabeled(
            RowKind::Hint,
            format!("no value '{}' to remove", parsed.as_ref()),
        )
    };
    Answer::of(
        vec![row],
        &json!({ KEY: parsed.as_ref(), DELETED: deleted }),
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf};

    use super::*;
    use crate::domain::{
        config::ConfigError,
        coordination::{
            Claim, KeyValue, Lock, LockAttempt, LockName, LockNote, LockRelease, LockTtl,
            Scratchpad, ScratchpadKey, Todo, TodoChange, TodoId, TodoLease, TodoTitle, TodoUpdate,
        },
    };

    /// An in-memory coordination store recording key-value entries for one project.
    #[derive(Default)]
    struct FakeStore {
        entries: RefCell<Vec<KeyValue>>,
    }

    impl CoordinationStore for FakeStore {
        fn values(&self, _project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError> {
            Ok(self.entries.borrow().clone())
        }

        fn value(
            &self,
            _project: &ProjectRoot,
            key: &KvKey,
        ) -> Result<Option<KeyValue>, ConfigError> {
            Ok(self
                .entries
                .borrow()
                .iter()
                .find(|entry| entry.key() == key)
                .cloned())
        }

        fn set_value(
            &self,
            _project: &ProjectRoot,
            author: &Author,
            key: &KvKey,
            value: &str,
        ) -> Result<(), ConfigError> {
            let entry = KeyValue::builder()
                .key(key.clone())
                .value(value.to_owned())
                .author(author.clone())
                .updated_at(0)
                .build();
            let mut entries = self.entries.borrow_mut();
            entries.retain(|existing| existing.key() != key);
            entries.push(entry);
            Ok(())
        }

        fn delete_value(&self, _project: &ProjectRoot, key: &KvKey) -> Result<bool, ConfigError> {
            let mut entries = self.entries.borrow_mut();
            let before = entries.len();
            entries.retain(|entry| entry.key() != key);
            Ok(entries.len() != before)
        }

        fn scratchpads(&self, _project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn scratchpad(
            &self,
            _project: &ProjectRoot,
            _key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn set_scratchpad(
            &self,
            _project: &ProjectRoot,
            _author: &Author,
            _key: &ScratchpadKey,
            _body: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn delete_scratchpad(
            &self,
            _project: &ProjectRoot,
            _key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn todos(&self, _project: &ProjectRoot) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn todo(&self, _project: &ProjectRoot, _id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn add_todo(
            &self,
            _project: &ProjectRoot,
            _author: &Author,
            _title: &TodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn update_todo(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _id: &TodoId,
            _change: &TodoChange,
        ) -> Result<TodoUpdate, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn claim_todo(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _id: Option<&TodoId>,
            _lease: TodoLease,
        ) -> Result<Claim, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn delete_todo(&self, _project: &ProjectRoot, _id: &TodoId) -> Result<bool, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn acquire_lock(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _name: &LockName,
            _ttl: LockTtl,
            _note: Option<&LockNote>,
        ) -> Result<LockAttempt, ConfigError> {
            unreachable!("kv commands never touch locks")
        }

        fn release_lock(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _name: &LockName,
        ) -> Result<LockRelease, ConfigError> {
            unreachable!("kv commands never touch locks")
        }

        fn break_lock(
            &self,
            _project: &ProjectRoot,
            _name: &LockName,
        ) -> Result<Option<Lock>, ConfigError> {
            unreachable!("kv commands never touch locks")
        }

        fn release_locks_of(&self, _me: &Author) -> Result<usize, ConfigError> {
            unreachable!("kv commands never touch locks")
        }

        fn locks(&self, _project: &ProjectRoot) -> Result<Vec<Lock>, ConfigError> {
            unreachable!("kv commands never touch locks")
        }

        fn import(
            &self,
            _project: &ProjectRoot,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("kv commands never import")
        }
    }

    fn project() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/repo")).unwrap()
    }

    /// Setting then listing shows `key  value`; getting returns the value.
    #[test]
    fn set_then_get_and_list() {
        let store = FakeStore::default();
        set(&store, &project(), &Author::human(), "port", "3000").unwrap();
        set(&store, &project(), &Author::human(), "port", "4000").unwrap();

        let listed = list(&store, &project()).unwrap();
        assert_eq!(
            listed.rows().len(),
            1,
            "the same key replaces, not duplicates"
        );
        assert_eq!(listed.rows()[0].detail(), "port  4000");
        assert_eq!(listed.json()["entries"][0]["value"], "4000");

        let value = get(&store, &project(), "port").unwrap();
        assert_eq!(value.rows()[0].detail(), "4000");
        assert_eq!(value.json()["key"], "port");
    }

    /// Getting an absent value errors so scripts see a non-zero exit.
    #[test]
    fn get_absent_is_an_error() {
        let store = FakeStore::default();
        assert!(matches!(
            get(&store, &project(), "ghost"),
            Err(CliError::UnknownValue(key)) if key == "ghost"
        ));
    }

    /// Removing succeeds once, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let store = FakeStore::default();
        set(&store, &project(), &Author::human(), "flag", "on").unwrap();
        assert_eq!(
            remove(&store, &project(), "flag").unwrap().rows()[0].kind(),
            RowKind::Ok
        );
        assert_eq!(
            remove(&store, &project(), "flag").unwrap().rows()[0].kind(),
            RowKind::Hint
        );
    }

    /// A blank key is rejected before the store is touched.
    #[test]
    fn a_blank_key_is_rejected() {
        let store = FakeStore::default();
        assert!(matches!(
            set(&store, &project(), &Author::human(), "   ", "v"),
            Err(CliError::InvalidKvKey(_))
        ));
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = FakeStore::default();
        assert_eq!(
            list(&store, &project()).unwrap().rows()[0].detail(),
            EMPTY_NOTE
        );
    }
}
