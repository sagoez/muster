use serde_json::json;

use super::{
    answer::Answer, error::CliError, note_command::NoteCommand, report::attribution, row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::{client::NoteEntry, wire::WireForm},
    domain::{
        coordination::{
            Author, NOTE_BODY_MAX_CHARS, NOTE_KEY_MAX_CHARS, NoteBody, ScratchpadKey,
            ScratchpadKeyError,
        },
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// What a refusal calls a note's body.
const NOTE_BODY: &str = "note body";
/// What a refusal calls a note's key.
const NOTE_KEY: &str = "note key";
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
/// Returns [`CliError::InvalidNoteKey`] when the key is blank or
/// whitespace-only, or [`CliError::TooLong`] past [`NOTE_KEY_MAX_CHARS`].
fn parse_key(key: &str) -> Result<ScratchpadKey, CliError> {
    ScratchpadKey::try_new(key).map_err(|error| match error {
        ScratchpadKeyError::LenCharMaxViolated => CliError::TooLong {
            what: NOTE_KEY,
            max: NOTE_KEY_MAX_CHARS,
            found: key.trim().chars().count(),
        },
        ScratchpadKeyError::NotEmptyViolated => CliError::InvalidNoteKey(key.to_string()),
    })
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
    use std::path::PathBuf;

    use super::{super::memory_coordination::MemoryCoordination, *};

    /// The project every test note belongs to.
    fn project() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/repo")).unwrap()
    }

    /// Setting then listing shows the key; getting returns the body verbatim.
    #[test]
    fn set_then_get_and_list() {
        let store = MemoryCoordination::default();
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
        let store = MemoryCoordination::default();
        let body = "x".repeat(NOTE_BODY_MAX_CHARS + 1);
        assert!(matches!(
            set(&store, &project(), &Author::human(), "plan", &body),
            Err(CliError::TooLong { max: NOTE_BODY_MAX_CHARS, found, .. })
                if found == NOTE_BODY_MAX_CHARS + 1
        ));
        assert!(store.scratchpads(&project()).unwrap().is_empty());
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = MemoryCoordination::default();
        let listed = list(&store, &project()).unwrap();
        assert_eq!(listed.rows()[0].detail(), EMPTY_NOTE);
        assert_eq!(listed.rows()[0].kind(), RowKind::Hint);
        assert_eq!(listed.json()["notes"], json!([]));
    }

    /// A note an agent wrote is attributed in the list; the human's is not.
    #[test]
    fn list_attributes_agent_notes() {
        let store = MemoryCoordination::default();
        set(&store, &project(), &Author::human(), "mine", "human note").unwrap();
        store
            .set_scratchpad(
                &project(),
                &Author::agent("claude"),
                &ScratchpadKey::try_new("theirs").unwrap(),
                &NoteBody::try_new("agent note").unwrap(),
            )
            .unwrap();

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
        let store = MemoryCoordination::default();
        assert!(matches!(
            get(&store, &project(), "ghost"),
            Err(CliError::UnknownNote(key)) if key == "ghost"
        ));
    }

    /// Removing reports success, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let store = MemoryCoordination::default();
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
        let store = MemoryCoordination::default();
        assert!(matches!(
            set(&store, &project(), &Author::human(), "   ", "body"),
            Err(CliError::InvalidNoteKey(_))
        ));
    }
}
