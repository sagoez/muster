use std::path::Path;

use super::{
    args::KvCommand,
    error::CliError,
    report::{Row, RowKind, attribution},
};
use crate::domain::{
    coordination::{Author, KvKey},
    port::CoordinationStore,
};

/// Line printed when the project has no key-value entries yet.
const EMPTY_NOTE: &str = "no values; write one with `muster kv set <key> <value>`";
/// Columns between a listed entry's key and value.
const FIELD_GAP: &str = "  ";

/// Runs a `muster kv` action against `store` for `project`, returning the rows to
/// print. Absent a subcommand, lists the project's entries.
///
/// # Errors
/// Returns [`CliError`] when a key is invalid, a requested value is absent, or the
/// store cannot be read or written.
pub fn kv(
    command: Option<KvCommand>,
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
) -> Result<Vec<Row>, CliError> {
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
fn list(store: &dyn CoordinationStore, project: &Path) -> Result<Vec<Row>, CliError> {
    let entries = store.values(project)?;
    if entries.is_empty() {
        return Ok(vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]);
    }
    Ok(entries
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
        .collect())
}

/// Prints the value under `key`, one row per line so the boxed layout stays
/// intact and piped output reproduces the value verbatim.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, [`CliError::UnknownValue`]
/// when no value is stored under it, or a store read failure.
fn get(store: &dyn CoordinationStore, project: &Path, key: &str) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    let entry = store
        .value(project, &parsed)?
        .ok_or_else(|| CliError::UnknownValue(key.to_string()))?;
    Ok(entry
        .value()
        .lines()
        .map(|line| Row::unlabeled(RowKind::Plain, line.to_string()))
        .collect())
}

/// Creates or replaces the entry under `key` with `value`.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, or a store write failure.
fn set(
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
    key: &str,
    value: &str,
) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    store.set_value(project, author, &parsed, value)?;
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        format!("saved '{}'", parsed.as_ref()),
    )])
}

/// Deletes the entry under `key`. Deleting an absent entry is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, or a store write failure.
fn remove(store: &dyn CoordinationStore, project: &Path, key: &str) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    if store.delete_value(project, &parsed)? {
        Ok(vec![Row::unlabeled(
            RowKind::Ok,
            format!("removed '{}'", parsed.as_ref()),
        )])
    } else {
        Ok(vec![Row::unlabeled(
            RowKind::Hint,
            format!("no value '{}' to remove", parsed.as_ref()),
        )])
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf};

    use super::*;
    use crate::domain::{
        config::ConfigError,
        coordination::{KeyValue, Scratchpad, ScratchpadKey, Todo, TodoId, TodoStatus, TodoTitle},
    };

    /// An in-memory coordination store recording key-value entries for one project.
    #[derive(Default)]
    struct FakeStore {
        entries: RefCell<Vec<KeyValue>>,
    }

    impl CoordinationStore for FakeStore {
        fn values(&self, _project: &Path) -> Result<Vec<KeyValue>, ConfigError> {
            Ok(self.entries.borrow().clone())
        }

        fn value(&self, _project: &Path, key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
            Ok(self
                .entries
                .borrow()
                .iter()
                .find(|entry| entry.key() == key)
                .cloned())
        }

        fn set_value(
            &self,
            _project: &Path,
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

        fn delete_value(&self, _project: &Path, key: &KvKey) -> Result<bool, ConfigError> {
            let mut entries = self.entries.borrow_mut();
            let before = entries.len();
            entries.retain(|entry| entry.key() != key);
            Ok(entries.len() != before)
        }

        fn scratchpads(&self, _project: &Path) -> Result<Vec<Scratchpad>, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn scratchpad(
            &self,
            _project: &Path,
            _key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn set_scratchpad(
            &self,
            _project: &Path,
            _author: &Author,
            _key: &ScratchpadKey,
            _body: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn delete_scratchpad(
            &self,
            _project: &Path,
            _key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("kv commands never touch scratchpads")
        }

        fn todos(&self, _project: &Path) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn todo(&self, _project: &Path, _id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn add_todo(
            &self,
            _project: &Path,
            _author: &Author,
            _title: &TodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn set_todo_status(
            &self,
            _project: &Path,
            _author: &Author,
            _id: &TodoId,
            _status: TodoStatus,
        ) -> Result<bool, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn delete_todo(&self, _project: &Path, _id: &TodoId) -> Result<bool, ConfigError> {
            unreachable!("kv commands never touch todos")
        }

        fn import(
            &self,
            _project: &Path,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("kv commands never import")
        }
    }

    fn project() -> PathBuf {
        PathBuf::from("/repo/muster.yml")
    }

    /// Setting then listing shows `key  value`; getting returns the value.
    #[test]
    fn set_then_get_and_list() {
        let store = FakeStore::default();
        set(&store, &project(), &Author::human(), "port", "3000").unwrap();
        set(&store, &project(), &Author::human(), "port", "4000").unwrap();

        let listed = list(&store, &project()).unwrap();
        assert_eq!(listed.len(), 1, "the same key replaces, not duplicates");
        assert_eq!(listed[0].detail(), "port  4000");

        let value = get(&store, &project(), "port").unwrap();
        assert_eq!(value[0].detail(), "4000");
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
            remove(&store, &project(), "flag").unwrap()[0].kind(),
            RowKind::Ok
        );
        assert_eq!(
            remove(&store, &project(), "flag").unwrap()[0].kind(),
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
        assert_eq!(list(&store, &project()).unwrap()[0].detail(), EMPTY_NOTE);
    }
}
