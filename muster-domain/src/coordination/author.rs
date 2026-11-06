use std::fmt;

use super::BlankAuthor;

/// Stored label for a write the human made through the CLI.
pub(crate) const HUMAN_LABEL: &str = "human";
/// Fallback label for an agent that did not name itself.
const DEFAULT_AGENT_LABEL: &str = "agent";

/// Who last wrote a coordination entry: the human at the CLI, or a named agent
/// connected over MCP. Persisted and surfaced in reads so agents can tell their
/// own entries from a peer's and the human can see what an agent left.
///
/// The agent name is a free-form label (not a validated newtype) so the human
/// and default variants construct infallibly; [`Author::agent`] guards it to a
/// non-empty value that cannot impersonate the human.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Author {
    /// The human, writing through the CLI.
    Human,
    /// A named agent, writing through the MCP server.
    Agent(String),
}

impl Author {
    /// The human writing through the CLI.
    #[must_use]
    pub const fn human() -> Self {
        Author::Human
    }

    /// A named agent. A blank name, or one impersonating the human label, falls
    /// back to a generic agent label.
    #[must_use]
    pub fn agent(name: &str) -> Self {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(HUMAN_LABEL) {
            Author::Agent(DEFAULT_AGENT_LABEL.to_string())
        } else {
            Author::Agent(trimmed.to_string())
        }
    }

    /// Whether this is the human writing through the CLI.
    #[must_use]
    pub const fn is_human(&self) -> bool {
        matches!(self, Author::Human)
    }

    /// Reconstructs an author from a stored or imported label - the inverse of
    /// [`Author::label`]. The label is trimmed and `human` matches in any case.
    ///
    /// # Errors
    /// Returns [`BlankAuthor`] when the label is blank once trimmed.
    pub fn from_label(label: &str) -> Result<Self, BlankAuthor> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            Err(BlankAuthor)
        } else if trimmed.eq_ignore_ascii_case(HUMAN_LABEL) {
            Ok(Author::Human)
        } else {
            Ok(Author::agent(trimmed))
        }
    }

    /// The flat label stored on disk and shown to readers.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Author::Human => HUMAN_LABEL,
            Author::Agent(name) => name,
        }
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The human label round-trips through its flat string form.
    #[test]
    fn the_human_round_trips() {
        assert_eq!(Author::human().label(), "human");
        assert_eq!(Author::Human.to_string(), "human");
        assert_eq!(Author::from_label("human"), Ok(Author::Human));
    }

    /// A named agent trims and round-trips.
    #[test]
    fn an_agent_name_trims_and_round_trips() {
        let author = Author::agent("  claude  ");
        assert_eq!(author.label(), "claude");
        assert_eq!(Author::from_label(&author.to_string()), Ok(author));
    }

    /// A label is trimmed and `human` matches in any case.
    #[test]
    fn a_respelt_human_label_reads_as_the_human() {
        assert_eq!(Author::from_label(" HUMAN "), Ok(Author::Human));
        assert_eq!(Author::from_label(" claude "), Ok(Author::agent("claude")));
    }

    /// A blank label names nobody and is refused.
    #[test]
    fn a_blank_label_is_refused() {
        assert_eq!(Author::from_label(""), Err(BlankAuthor));
        assert_eq!(Author::from_label("  \t"), Err(BlankAuthor));
    }

    /// A blank name, or one impersonating the human, collapses to the generic
    /// agent label.
    #[test]
    fn a_blank_or_impersonating_name_falls_back() {
        assert_eq!(Author::agent("   ").label(), "agent");
        assert_eq!(Author::agent("Human").label(), "agent");
    }
}
