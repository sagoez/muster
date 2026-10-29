use serde_json::json;

use super::{
    answer::Answer, error::CliError, note_command::NoteCommand, report::attribution, row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::{client::NoteEntry, wire::WireForm},
    domain::{
        coordination::{Author, NOTE_BODY_MAX_CHARS, NoteBody, ScratchpadKey},
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// What a refusal calls a note's body.
const NOTE_BODY: &str = "note body";
/// Line printed when the project has no notes yet.
const EMPTY_NOTE: &str = "no notes; write one with `muster note set <key> <body>`";
/// The JSON key of the listed notes.
const NOTES: &str = "notes";
/// The JSON key of the key a write saved.
const SAVED: &str = "saved";
/// The JSON key of the key a delete named.
const KEY: &str = "key";
/// The JSON key of whether a delete found something.
const DELETED: &str = "deleted";

/// Runs a `muster note` action against `store` for `project`, returning what
/// to print. Absent a subcommand, lists the project's notes.
///
/// # Errors
/// Returns [`CliError`] when a key is invalid, a requested note is absent, or the
/// store cannot be read or written.
pub fn note(
    command: Option<NoteCommand>,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
) -> Result<Answer, CliError> {
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

/// Lists the keys of every note stored for `project`, one per line, without
/// their bodies.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(store: &dyn CoordinationStore, project: &ProjectRoot) -> Result<Answer, CliError> {
    let notes = store.scratchpads(project)?;
    let entries: Vec<NoteEntry> = notes.iter().map(NoteEntry::from).collect();
    let rows = if notes.is_empty() {
        vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]
    } else {
        notes
            .iter()
            .map(|note| {
                Row::unlabeled(
                    RowKind::Plain,
                    format!("{}{}", note.key().as_ref(), attribution(note.author())),
                )
            })
            .collect()
    };
    Answer::of(rows, &json!({ NOTES: entries }))
}

/// Prints the body of the note under `key`, one row per line so the framed
/// layout stays intact and piped output reproduces the body verbatim.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key, [`CliError::UnknownNote`]
/// when no note is stored under it, or a store read failure.
fn get(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let note = store
        .scratchpad(project, &parsed)?
        .ok_or_else(|| CliError::UnknownNote(key.to_string()))?;
    let rows = note
        .body()
        .lines()
        .map(|line| Row::unlabeled(RowKind::Plain, line.to_string()))
        .collect();
    Answer::of(rows, &note.to_wire())
}

/// Creates or replaces the note under `key` with `body`.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key,
/// [`CliError::TooLong`] for a body past [`NOTE_BODY_MAX_CHARS`], or a store
/// write failure.
fn set(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
    key: &str,
    body: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let body = NoteBody::try_new(body).map_err(|_| CliError::TooLong {
        what: NOTE_BODY,
        max: NOTE_BODY_MAX_CHARS,
        found: body.chars().count(),
    })?;
    store.set_scratchpad(project, author, &parsed, &body)?;
    Answer::of(
        vec![Row::unlabeled(
            RowKind::Ok,
            format!("saved '{}'", parsed.as_ref()),
        )],
        &json!({ SAVED: parsed.as_ref() }),
    )
}

/// Deletes the note under `key`. Deleting an absent note is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidNoteKey`] for a blank key, or a store write failure.
fn remove(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let deleted = store.delete_scratchpad(project, &parsed)?;
    let row = if deleted {
        Row::unlabeled(RowKind::Ok, format!("removed '{}'", parsed.as_ref()))
    } else {
        Row::unlabeled(
            RowKind::Hint,
            format!("no note '{}' to remove", parsed.as_ref()),
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
            Claim, KeyValue, KvKey, KvValue, Lock, LockAttempt, LockHolder, LockName, LockNote,
            LockRelease, LockTtl, NewTodoTitle, Scratchpad, SessionId, Todo, TodoChange, TodoId,
            TodoLease, TodoUpdate,
        },
        value::EpochSeconds,
    };

    /// An in-memory coordination store recording scratchpads for one project.
    #[derive(Default)]
    struct FakeStore {
        notes: RefCell<Vec<Scratchpad>>,
    }

    impl CoordinationStore for FakeStore {
        fn scratchpads(&self, _project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError> {
            Ok(self.notes.borrow().clone())
        }

        fn scratchpad(
            &self,
            _project: &ProjectRoot,
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
            _project: &ProjectRoot,
            author: &Author,
            key: &ScratchpadKey,
            body: &NoteBody,
        ) -> Result<(), ConfigError> {
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_string())
                .author(author.clone())
                .updated_at(EpochSeconds::new(0))
                .build();
            let mut notes = self.notes.borrow_mut();
            notes.retain(|existing| existing.key() != key);
            notes.push(note);
            Ok(())
        }

        fn delete_scratchpad(
            &self,
            _project: &ProjectRoot,
            key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            let mut notes = self.notes.borrow_mut();
            let before = notes.len();
            notes.retain(|note| note.key() != key);
            Ok(notes.len() != before)
        }

        fn todos(&self, _project: &ProjectRoot) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn todo(&self, _project: &ProjectRoot, _id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn add_todo(
            &self,
            _project: &ProjectRoot,
            _author: &Author,
            _title: &NewTodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn update_todo(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _id: &TodoId,
            _change: &TodoChange,
        ) -> Result<TodoUpdate, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn claim_todo(
            &self,
            _project: &ProjectRoot,
            _me: &Author,
            _id: Option<&TodoId>,
            _lease: TodoLease,
        ) -> Result<Claim, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn delete_todo(&self, _project: &ProjectRoot, _id: &TodoId) -> Result<bool, ConfigError> {
            unreachable!("note commands never touch todos")
        }

        fn acquire_lock(
            &self,
            _project: &ProjectRoot,
            _me: &LockHolder,
            _name: &LockName,
            _ttl: LockTtl,
            _note: Option<&LockNote>,
        ) -> Result<LockAttempt, ConfigError> {
            unreachable!("note commands never touch locks")
        }

        fn release_lock(
            &self,
            _project: &ProjectRoot,
            _me: &LockHolder,
            _name: &LockName,
        ) -> Result<LockRelease, ConfigError> {
            unreachable!("note commands never touch locks")
        }

        fn break_lock(
            &self,
            _project: &ProjectRoot,
            _name: &LockName,
        ) -> Result<Option<Lock>, ConfigError> {
            unreachable!("note commands never touch locks")
        }

        fn release_locks_of(&self, _session: &SessionId) -> Result<usize, ConfigError> {
            unreachable!("note commands never touch locks")
        }

        fn locks(&self, _project: &ProjectRoot) -> Result<Vec<Lock>, ConfigError> {
            unreachable!("note commands never touch locks")
        }

        fn values(&self, _project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn value(
            &self,
            _project: &ProjectRoot,
            _key: &KvKey,
        ) -> Result<Option<KeyValue>, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn set_value(
            &self,
            _project: &ProjectRoot,
            _author: &Author,
            _key: &KvKey,
            _value: &KvValue,
        ) -> Result<(), ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn delete_value(&self, _project: &ProjectRoot, _key: &KvKey) -> Result<bool, ConfigError> {
            unreachable!("note commands never touch key-values")
        }

        fn import(
            &self,
            _project: &ProjectRoot,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("note commands never import")
        }
    }

    fn project() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/repo")).unwrap()
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
        assert_eq!(listed.rows().len(), 1);
        assert_eq!(listed.rows()[0].detail(), "plan");
        assert_eq!(listed.json()["notes"][0]["key"], "plan");
        assert!(listed.json()["notes"][0].get("body").is_none());

        let body = get(&store, &project(), "plan").unwrap();
        assert_eq!(body.rows().len(), 2);
        assert_eq!(body.rows()[0].detail(), "line one");
        assert_eq!(body.rows()[1].detail(), "line two");
        assert_eq!(body.json()["body"], "line one\nline two");
    }

    /// A body past the maximum is refused with its length, and not stored.
    #[test]
    fn an_oversized_body_is_refused() {
        let store = FakeStore::default();
        let body = "x".repeat(NOTE_BODY_MAX_CHARS + 1);
        assert!(matches!(
            set(&store, &project(), &Author::human(), "plan", &body),
            Err(CliError::TooLong { max: NOTE_BODY_MAX_CHARS, found, .. })
                if found == NOTE_BODY_MAX_CHARS + 1
        ));
        assert!(store.notes.borrow().is_empty());
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = FakeStore::default();
        let listed = list(&store, &project()).unwrap();
        assert_eq!(listed.rows()[0].detail(), EMPTY_NOTE);
        assert_eq!(listed.rows()[0].kind(), RowKind::Hint);
        assert_eq!(listed.json()["notes"], json!([]));
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
                .updated_at(EpochSeconds::new(0))
                .build(),
        );

        let listed = list(&store, &project()).unwrap();
        let details: Vec<&str> = listed
            .rows()
            .iter()
            .map(|row| row.detail().as_str())
            .collect();
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
        assert_eq!(first.rows()[0].kind(), RowKind::Ok);
        assert_eq!(first.json(), &json!({"key": "plan", "deleted": true}));
        let second = remove(&store, &project(), "plan").unwrap();
        assert_eq!(second.rows()[0].kind(), RowKind::Hint);
        assert_eq!(second.json()["deleted"], false);
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
