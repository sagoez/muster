use strum::{EnumIter, EnumProperty};

use crate::cli::mcp::context::bounds::{
    CONTEXT_MAX_BYTES, LOCKS_MAX_BYTES, MIN_ENTRY_BYTES, MIN_NOTE_BYTES, PROJECTS_MAX_BYTES,
    TODOS_MAX_BYTES,
};

/// The property holding the line that heads a listing.
const HEADING: &str = "heading";
/// The property holding what follows the count of entries left out for
/// room, and their noun.
const MORE: &str = "more";
/// The property holding what one entry is called in a count.
const ONE: &str = "one";
/// The property holding what more entries are called in a count.
const MANY: &str = "many";

/// One of the context's listings, its words held as properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumProperty)]
pub enum Listing {
    /// The open todos, oldest first.
    #[strum(props(
        heading = "Open todos, oldest first:",
        more = "left out for room; todo_list has them]",
        one = "open todo",
        many = "open todos"
    ))]
    Todos,
    /// The held locks, by name.
    #[strum(props(
        heading = "Held locks, by name:",
        more = "left out for room; stack_status lists them]",
        one = "lock",
        many = "locks"
    ))]
    Locks,
    /// Every registered project, this one first, which a note, todo or
    /// lock reaches by passing `project`.
    #[strum(props(
        heading = "Registered projects, often related repositories in other folders; note, \
                   todo and lock tools reach one by passing project, its name:",
        more = "left out for room; project_list lists them]",
        one = "project",
        many = "projects"
    ))]
    Projects,
    /// The notes, newest first.
    #[strum(props(
        heading = "Notes, newest first:",
        more = "left out for room; note_read lists them]",
        one = "note",
        many = "notes"
    ))]
    Notes,
}

impl Listing {
    /// The line heading the listing.
    pub fn heading(self) -> &'static str {
        self.get_str(HEADING).unwrap_or_default()
    }

    /// The most bytes the listing takes of the context; the notes take what
    /// the others leave.
    pub fn max_bytes(self) -> usize {
        match self {
            Self::Todos => TODOS_MAX_BYTES,
            Self::Locks => LOCKS_MAX_BYTES,
            Self::Projects => PROJECTS_MAX_BYTES,
            Self::Notes => CONTEXT_MAX_BYTES,
        }
    }

    /// The fewest bytes one of its entries takes.
    fn min_entry_bytes(self) -> usize {
        match self {
            Self::Notes => MIN_NOTE_BYTES,
            Self::Todos | Self::Locks | Self::Projects => MIN_ENTRY_BYTES,
        }
    }

    /// The most rows tried for it, however many fail to fit: more than its
    /// room could ever show.
    pub fn limit(self) -> usize {
        self.max_bytes() / self.min_entry_bytes()
    }

    /// What follows the count of entries left out for room, and its noun.
    pub fn more(self) -> &'static str {
        self.get_str(MORE).unwrap_or_default()
    }

    /// What one entry, and more, are called in the lines counting them.
    pub fn nouns(self) -> (&'static str, &'static str) {
        (
            self.get_str(ONE).unwrap_or_default(),
            self.get_str(MANY).unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Each listing has every word it shows: a heading, what was left out,
    /// and its nouns.
    #[test]
    fn each_listing_has_its_words() {
        for listing in Listing::iter() {
            let (one, many) = listing.nouns();
            assert!(!listing.heading().is_empty() && !listing.more().is_empty());
            assert!(!one.is_empty() && many.starts_with(one), "{one} {many}");
        }
    }
}
