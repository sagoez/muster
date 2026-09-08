use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// Stored label for a write the human made through the CLI.
const HUMAN_LABEL: &str = "human";
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

    /// Reconstructs an author from a stored flat label - the inverse of
    /// [`Author::label`], for reading back persisted entries.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        if label == HUMAN_LABEL {
            Author::Human
        } else {
            Author::agent(label)
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

impl Serialize for Author {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for Author {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(D::Error::custom("author label must not be empty"));
        }
        Ok(if trimmed == HUMAN_LABEL {
            Author::Human
        } else {
            Author::Agent(trimmed.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The human label round-trips through its flat string form.
    #[test]
    fn the_human_round_trips() {
        assert_eq!(Author::human().label(), "human");
        let json = serde_json::to_string(&Author::Human).unwrap();
        assert_eq!(json, "\"human\"");
        assert_eq!(
            serde_json::from_str::<Author>(&json).unwrap(),
            Author::Human
        );
    }

    /// A named agent trims and round-trips.
    #[test]
    fn an_agent_name_trims_and_round_trips() {
        let author = Author::agent("  claude  ");
        assert_eq!(author.label(), "claude");
        let json = serde_json::to_string(&author).unwrap();
        assert_eq!(serde_json::from_str::<Author>(&json).unwrap(), author);
    }

    /// A blank name, or one impersonating the human, collapses to the generic
    /// agent label.
    #[test]
    fn a_blank_or_impersonating_name_falls_back() {
        assert_eq!(Author::agent("   ").label(), "agent");
        assert_eq!(Author::agent("Human").label(), "agent");
    }
}
