use std::path::Path;

use super::{
    args::NoteCommand,
    error::CliError,
    report::{Row, RowKind, attribution},
};
use crate::domain::{
    coordination::{Author, ScratchpadKey},
    port::CoordinationStore,
};

/// Line printed when the project has no notes yet.
const EMPTY_NOTE: &str = "no notes; write one with `muster note set <key> <body>`";

/// Runs a `muster note` action against `store` for `project`, returning the rows
/// to print. Absent a subcommand, lists the project's notes.
///
/// # Errors
/// Returns [`CliError`] when a key is invalid, a requested note is absent, or the
/// store cannot be read or written.
pub fn note(
    command: Option<NoteCommand>,
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
) -> Result<Vec<Row>, CliError> {
    match command {
        None => list(store, project),
        Some(NoteCommand::Get { key }) => get(store, project, &key),
        Some(NoteCommand::Set { key, body }) => set(store, project, author, &key, &body),
        Some(NoteCommand::Rm { key }) => remove(store, project, &key),
    }
}

/// Parses a user-supplied note key into a validated [`ScratchpadKey`].
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] when the key is blank or whitespace-only.
fn parse_key(key: &str) -> Result<ScratchpadKey, CliError> {
    ScratchpadKey::try_new(key).map_err(|_| CliError::InvalidNoteKey(key.to_string()))
}

/// Lists the keys of every note stored for `project`, one per line.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(store: &dyn CoordinationStore, project: &Path) -> Result<Vec<Row>, CliError> {
    let notes = store.scratchpads(project)?;
    if notes.is_empty() {
        return Ok(vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]);
    }
    Ok(notes
        .iter()
        .map(|note| {
            Row::unlabeled(
                RowKind::Plain,
                format!("{}{}", note.key().as_ref(), attribution(note.author())),
            )
        })
        .collect())
}

/// Prints the body of the note under `key`, one row per line so the boxed layout
/// stays intact and piped output reproduces the body verbatim.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key, [`CliError::UnknownNote`]
/// when no note is stored under it, or a store read failure.
fn get(store: &dyn CoordinationStore, project: &Path, key: &str) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    let note = store
        .scratchpad(project, &parsed)?
        .ok_or_else(|| CliError::UnknownNote(key.to_string()))?;
    Ok(note
        .body()
        .lines()
        .map(|line| Row::unlabeled(RowKind::Plain, line.to_string()))
        .collect())
}

/// Creates or replaces the note under `key` with `body`.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key, or a store write failure.
fn set(
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
    key: &str,
    body: &str,
) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    store.set_scratchpad(project, author, &parsed, body)?;
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        format!("saved '{}'", parsed.as_ref()),
    )])
}

/// Deletes the note under `key`. Deleting an absent note is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key, or a store write failure.
fn remove(store: &dyn CoordinationStore, project: &Path, key: &str) -> Result<Vec<Row>, CliError> {
    let parsed = parse_key(key)?;
    if store.delete_scratchpad(project, &parsed)? {
        Ok(vec![Row::unlabeled(
            RowKind::Ok,
            format!("removed '{}'", parsed.as_ref()),
        )])
    } else {
        Ok(vec![Row::unlabeled(
            RowKind::Hint,
            format!("no note '{}' to remove", parsed.as_ref()),
        )])
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf};

    use super::*;
    use crate::domain::{
        config::ConfigError,
        coordination::{KeyValue, KvKey, Scratchpad, Todo, TodoId, TodoStatus, TodoTitle},
    };

    /// An in-memory coordination store recording scratchpads for one project.
    #[derive(Default)]
    struct FakeStore {
        notes: RefCell<Vec<Scratchpad>>,
    }

    impl CoordinationStore for FakeStore {
        fn scratchpads(&self, _project: &Path) -> Result<Vec<Scratchpad>, ConfigError> {
            Ok(self.notes.borrow().clone())
        }

        fn scratchpad(
            &self,
            _project: &Path,
            key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            Ok(self
                .notes
                .borrow()
                .iter()
                .find(|note| note.key() == key)
                .cloned())
        }

        fn set_scratchpad(
            &self,
            _project: &Path,
            author: &Author,
            key: &ScratchpadKey,
            body: &str,
        ) -> Result<(), ConfigError> {
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_owned())
                .author(author.clone())
                .updated_at(0)
                .build();
            let mut notes = self.notes.borrow_mut();
            notes.retain(|existing| existing.key() != key);
            notes.push(note);
            Ok(())
        }

        fn delete_scratchpad(
            &self,
            _project: &Path,
            key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            let mut notes = self.notes.borrow_mut();
            let before = notes.len();
            notes.retain(|note| note.key() != key);
            Ok(notes.len() != before)
        }

        fn todos(&self, _project: &Path) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn todo(&self, _project: &Path, _id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn add_todo(
            &self,
            _project: &Path,
            _author: &Author,
            _title: &TodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn set_todo_status(
            &self,
            _project: &Path,
            _author: &Author,
            _id: &TodoId,
            _status: TodoStatus,
        ) -> Result<bool, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn delete_todo(&self, _project: &Path, _id: &TodoId) -> Result<bool, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn values(&self, _project: &Path) -> Result<Vec<KeyValue>, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn value(&self, _project: &Path, _key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn set_value(
            &self,
            _project: &Path,
            _author: &Author,
            _key: &KvKey,
            _value: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn delete_value(&self, _project: &Path, _key: &KvKey) -> Result<bool, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn import(
            &self,
            _project: &Path,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("note commands never import")
        }
    }

    fn project() -> PathBuf {
        PathBuf::from("/repo/muster.yml")
    }

    /// Setting then listing shows the key; getting returns the body verbatim.
    #[test]
    fn set_then_get_and_list() {
        let store = FakeStore::default();
        set(
            &store,
            &project(),
            &Author::human(),
            "plan",
            "line one\nline two",
        )
        .unwrap();

        let listed = list(&store, &project()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].detail(), "plan");

        let body = get(&store, &project(), "plan").unwrap();
        assert_eq!(body.len(), 2);
        assert_eq!(body[0].detail(), "line one");
        assert_eq!(body[1].detail(), "line two");
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = FakeStore::default();
        let rows = list(&store, &project()).unwrap();
        assert_eq!(rows[0].detail(), EMPTY_NOTE);
        assert_eq!(rows[0].kind(), RowKind::Hint);
    }

    /// A note an agent wrote is attributed in the list; the human's is not.
    #[test]
    fn list_attributes_agent_notes() {
        let store = FakeStore::default();
        set(&store, &project(), &Author::human(), "mine", "human note").unwrap();
        store.notes.borrow_mut().push(
            Scratchpad::builder()
                .key(ScratchpadKey::try_new("theirs").unwrap())
                .body("agent note".to_string())
                .author(Author::agent("claude"))
                .updated_at(0)
                .build(),
        );

        let rows = list(&store, &project()).unwrap();
        let details: Vec<&str> = rows.iter().map(|row| row.detail().as_str()).collect();
        assert!(details.contains(&"mine"), "human note is unadorned");
        assert!(
            details.contains(&"theirs  (by claude)"),
            "agent note is attributed: {details:?}"
        );
    }

    /// Getting an absent note errors so scripts see a non-zero exit.
    #[test]
    fn get_absent_is_an_error() {
        let store = FakeStore::default();
        assert!(matches!(
            get(&store, &project(), "ghost"),
            Err(CliError::UnknownNote(key)) if key == "ghost"
        ));
    }

    /// Removing reports success, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let store = FakeStore::default();
        set(&store, &project(), &Author::human(), "plan", "body").unwrap();

        let first = remove(&store, &project(), "plan").unwrap();
        assert_eq!(first[0].kind(), RowKind::Ok);
        let second = remove(&store, &project(), "plan").unwrap();
        assert_eq!(second[0].kind(), RowKind::Hint);
    }

    /// A blank key is rejected before the store is touched.
    #[test]
    fn a_blank_key_is_rejected() {
        let store = FakeStore::default();
        assert!(matches!(
            set(&store, &project(), &Author::human(), "   ", "body"),
            Err(CliError::InvalidNoteKey(_))
        ));
    }
}
