use super::WireForm;
use crate::domain::coordination::Author;

/// Why a blank author label is refused: it names nobody.
const BLANK_AUTHOR: &str = "author label must not be empty";

impl WireForm for Author {
    type Wire = String;

    /// The author's stored label.
    fn to_wire(&self) -> String {
        self.label().to_string()
    }

    /// The author a label names, read as the store reads it.
    fn from_wire(wire: String) -> Result<Self, String> {
        if wire.trim().is_empty() {
            return Err(BLANK_AUTHOR.to_string());
        }
        Ok(Self::from_label(&wire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            BLANK_AUTHOR
        );
    }

    /// A label that only spells the human differently reads as an agent, as
    /// the store reads it, so an imported entry cannot pass for the human.
    #[test]
    fn a_respelt_human_label_reads_as_an_agent() {
        assert_eq!(
            Author::from_wire(" HUMAN ".to_string()).unwrap(),
            Author::agent("agent")
        );
    }
}
