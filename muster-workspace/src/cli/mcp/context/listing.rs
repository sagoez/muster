use crate::cli::mcp::context::bounds::{CONTEXT_MAX_BYTES, MIN_ENTRY_BYTES, TODOS_MAX_BYTES};

/// Heads the open todos.
const TODOS_HEADING: &str = "Open todos, oldest first:";
/// Heads the notes.
const NOTES_HEADING: &str = "Notes, newest first:";
/// Follows the count of open todos left out.
const MORE_TODOS: &str = "open todos left out for room; todo_list has them]";
/// Follows the count of notes left out.
const MORE_NOTES: &str = "notes left out for room; note_read lists them]";
/// One open todo that could not be read, and more.
const TODO_NOUNS: (&str, &str) = ("open todo", "open todos");
/// One note that could not be read, and more.
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

    /// What follows the count of entries left out for room.
    pub fn more(self) -> &'static str {
        match self {
            Self::Todos => MORE_TODOS,
            Self::Notes => MORE_NOTES,
        }
    }

    /// What one entry, and more, are called in the line counting the rows
    /// that could not be read.
    pub fn nouns(self) -> (&'static str, &'static str) {
        match self {
            Self::Todos => TODO_NOUNS,
            Self::Notes => NOTE_NOUNS,
        }
    }
}
