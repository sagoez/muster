use strum::EnumProperty;

use crate::cli::mcp::context::bounds::{
    CONTEXT_MAX_BYTES, MIN_ENTRY_BYTES, MIN_NOTE_BYTES, TODOS_MAX_BYTES,
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

/// One of the context's two listings, its words held as properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumProperty)]
pub enum Listing {
    /// The open todos, oldest first.
    #[strum(props(
        heading = "Open todos, oldest first:",
        more = "left out for room; todo_list has them]",
        one = "open todo",
        many = "open todos"
    ))]
    Todos,
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

    /// The most rows tried for it, however many fail to fit: more than its
    /// room could ever show.
    pub fn limit(self) -> usize {
        match self {
            Self::Todos => TODOS_MAX_BYTES / MIN_ENTRY_BYTES,
            Self::Notes => CONTEXT_MAX_BYTES / MIN_NOTE_BYTES,
        }
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
    use super::*;

    /// Each listing has every word it shows: a heading, what was left out,
    /// and its nouns.
    #[test]
    fn each_listing_has_its_words() {
        for listing in [Listing::Todos, Listing::Notes] {
            let (one, many) = listing.nouns();
            assert!(!listing.heading().is_empty() && !listing.more().is_empty());
            assert!(!one.is_empty() && many.starts_with(one), "{one} {many}");
        }
    }
}
