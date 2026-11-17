use crate::cli::mcp::context::bounds::{CONTEXT_MAX_BYTES, MIN_ENTRY_BYTES, TODOS_MAX_BYTES};

/// Heads the open todos.
const TODOS_HEADING: &str = "Open todos, oldest first:";
/// Heads the notes.
const NOTES_HEADING: &str = "Notes, newest first:";
/// Follows the count of open todos left out, and their noun.
const MORE_TODOS: &str = "left out for room; todo_list has them]";
/// Follows the count of notes left out, and their noun.
const MORE_NOTES: &str = "left out for room; note_read lists them]";
/// One open todo, and more, as a count names them.
const TODO_NOUNS: (&str, &str) = ("open todo", "open todos");
/// One note, and more, as a count names them.
const NOTE_NOUNS: (&str, &str) = ("note", "notes");

/// One of the context's two listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing {
    /// The open todos, oldest first.
    Todos,
    /// The notes, newest first.
    Notes,
}

impl Listing {
    /// The line heading the listing.
    pub fn heading(self) -> &'static str {
        match self {
            Self::Todos => TODOS_HEADING,
            Self::Notes => NOTES_HEADING,
        }
    }

    /// The most rows tried for it, however many fail to fit: more than its
    /// room could ever show.
    pub fn limit(self) -> usize {
        match self {
            Self::Todos => TODOS_MAX_BYTES / MIN_ENTRY_BYTES,
            Self::Notes => CONTEXT_MAX_BYTES / MIN_ENTRY_BYTES,
        }
    }

    /// What follows the count of entries left out for room, and its noun.
    pub fn more(self) -> &'static str {
        match self {
            Self::Todos => MORE_TODOS,
            Self::Notes => MORE_NOTES,
        }
    }

    /// What one entry, and more, are called in the lines counting them.
    pub fn nouns(self) -> (&'static str, &'static str) {
        match self {
            Self::Todos => TODO_NOUNS,
            Self::Notes => NOTE_NOUNS,
        }
    }
}
