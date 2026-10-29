use super::WireForm;
use crate::domain::coordination::Author;

impl WireForm for Author {
    type Wire = String;

    /// The author's stored label.
    fn to_wire(&self) -> String {
        self.label().to_string()
    }

    /// The author a label names, read as the store reads it.
    fn from_wire(wire: String) -> Result<Self, String> {
        Self::from_label(&wire).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coordination::BlankAuthor;

    /// The human and a named agent round-trip through their labels.
    #[test]
    fn an_author_round_trips_through_its_label() {
        for author in [Author::human(), Author::agent("claude")] {
            assert_eq!(Author::from_wire(author.to_wire()).unwrap(), author);
        }
    }

    /// A blank label names nobody and is refused.
    #[test]
    fn a_blank_label_is_refused() {
        assert_eq!(
            Author::from_wire("  ".to_string()).unwrap_err(),
            BlankAuthor.to_string()
        );
    }

    /// A label that spells the human in another case, padded, reads as the
    /// human, as the store reads it.
    #[test]
    fn a_respelt_human_label_reads_as_the_human() {
        assert_eq!(
            Author::from_wire(" HUMAN ".to_string()).unwrap(),
            Author::Human
        );
    }
}
