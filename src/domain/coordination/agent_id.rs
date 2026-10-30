use std::fmt;

use uuid::Uuid;

use crate::domain::coordination::{Author, HUMAN_LABEL};

/// The flag-free environment variable naming an agent explicitly.
pub const MUSTER_AGENT_ENV: &str = "MUSTER_AGENT";
/// The environment variable herdr sets to the pane an agent runs in.
pub const HERDR_PANE_ENV: &str = "HERDR_PANE_ID";
/// The variables herdr sets to name the one pane, tab and workspace a shell
/// runs in. A process muster starts outlives that pane and must not claim
/// it, so no launch carries them.
pub const PANE_IDENTITY_ENV: [&str; 4] = [
    HERDR_PANE_ENV,
    "HERDR_PANE_RUNTIME_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];
/// Prefix of an identity derived from a herdr pane id.
const HERDR_PANE_PREFIX: &str = "herdr-pane-";
/// Prefix of the random identity an unnamed MCP server falls back to.
const SESSION_PREFIX: &str = "session-";
/// Hex digits of the random session identity.
const SESSION_HEX_DIGITS: usize = 6;
/// Joins the client name to the identity in the stored label.
const CLIENT_SEPARATOR: char = '@';
/// Stands in for every character an identity part may not hold.
const SLUG_FILL: char = '-';
/// Longest identity part kept, in characters, so a label stays bounded.
const PART_MAX_CHARS: usize = 64;
/// Punctuation an identity part may hold besides ASCII letters and digits.
const PART_PUNCTUATION: &[char] = &['.', '_', '-', ':'];

/// The owner label an MCP connection writes coordination entries under:
/// `<client>@<id>`, or `<id>` when the client gave no usable name. Every part
/// is reduced to letters, digits and `._-:`, so a label is bounded, printable
/// and can never read as the human.
///
/// Hand-written rather than `nutype`: every input is slugged into a valid
/// label, so construction never fails and a fallible constructor would only
/// add an unreachable error.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentId(String);

impl AgentId {
    /// Resolves a connection's identity, first match wins: `flag` (`--as`),
    /// [`MUSTER_AGENT_ENV`], `herdr-pane-<HERDR_PANE_ID>`, then a random
    /// `session-<hex>`. `client` is the MCP `clientInfo.name`, prefixed when
    /// it slugs to something.
    #[must_use]
    pub fn resolve(
        flag: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
        client: Option<&str>,
    ) -> Self {
        let herdr = || {
            env(HERDR_PANE_ENV)
                .as_deref()
                .and_then(slug)
                .map(|pane| format!("{HERDR_PANE_PREFIX}{pane}"))
        };
        let id = flag
            .and_then(slug)
            .or_else(|| env(MUSTER_AGENT_ENV).as_deref().and_then(slug))
            .or_else(herdr)
            .unwrap_or_else(session_id);
        match client.and_then(slug) {
            Some(client) => Self(format!("{client}{CLIENT_SEPARATOR}{id}")),
            None => Self(id),
        }
    }

    /// The author this identity writes coordination entries as.
    #[must_use]
    pub fn author(&self) -> Author {
        Author::agent(&self.0)
    }
}

impl AsRef<str> for AgentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `c` may stay in an identity part.
fn is_part_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || PART_PUNCTUATION.contains(&c)
}

/// `raw` reduced to an identity part: disallowed characters become
/// [`SLUG_FILL`], the result is cut to [`PART_MAX_CHARS`] and trimmed of fill.
/// `None` when nothing is left or it would read as the human.
fn slug(raw: &str) -> Option<String> {
    let filled: String = raw
        .trim()
        .chars()
        .take(PART_MAX_CHARS)
        .map(|c| if is_part_char(c) { c } else { SLUG_FILL })
        .collect();
    let part = filled.trim_matches(SLUG_FILL);
    (!part.is_empty() && !part.eq_ignore_ascii_case(HUMAN_LABEL)).then(|| part.to_string())
}

/// A random `session-<hex>` identity, unique per MCP server process.
fn session_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("{SESSION_PREFIX}{}", &hex[..SESSION_HEX_DIGITS])
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// An environment lookup over `vars`.
    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    /// The flag beats the environment, which beats the herdr pane.
    #[test]
    fn the_first_source_wins() {
        let env = env_of(&[(MUSTER_AGENT_ENV, "reviewer"), (HERDR_PANE_ENV, "3")]);
        assert_eq!(AgentId::resolve(Some("lead"), &env, None).as_ref(), "lead");
        assert_eq!(AgentId::resolve(None, &env, None).as_ref(), "reviewer");
        let herdr = env_of(&[(HERDR_PANE_ENV, "3")]);
        assert_eq!(
            AgentId::resolve(None, &herdr, None).as_ref(),
            "herdr-pane-3"
        );
    }

    /// With nothing to go on, the identity is a random session id.
    #[test]
    fn without_a_source_a_session_id_is_minted() {
        let none = env_of(&[]);
        let one = AgentId::resolve(None, &none, None);
        let two = AgentId::resolve(None, &none, None);
        assert!(one.as_ref().starts_with(SESSION_PREFIX));
        assert_eq!(
            one.as_ref().len(),
            SESSION_PREFIX.len() + SESSION_HEX_DIGITS
        );
        assert_ne!(one, two);
    }

    /// The client name prefixes the identity.
    #[test]
    fn the_client_prefixes_the_label() {
        let env = env_of(&[(HERDR_PANE_ENV, "3")]);
        assert_eq!(
            AgentId::resolve(None, &env, Some("claude-code")).as_ref(),
            "claude-code@herdr-pane-3"
        );
    }

    /// Spaces, control characters and `@` are slugged; a blank or human source
    /// is skipped for the next one.
    #[test]
    fn untrusted_parts_are_slugged_or_skipped() {
        let env = env_of(&[(MUSTER_AGENT_ENV, "  "), (HERDR_PANE_ENV, "7")]);
        assert_eq!(
            AgentId::resolve(Some("Human"), &env, Some("Claude Desktop\u{1b}[31m")).as_ref(),
            "Claude-Desktop--31m@herdr-pane-7"
        );
        assert_eq!(
            AgentId::resolve(Some("a@b"), &env, Some("@@")).as_ref(),
            "a-b"
        );
    }

    /// A long part is cut, so the label stays bounded.
    #[test]
    fn a_long_part_is_cut() {
        let long = "x".repeat(PART_MAX_CHARS * 2);
        let id = AgentId::resolve(Some(&long), &env_of(&[]), Some(&long));
        assert_eq!(id.as_ref().len(), PART_MAX_CHARS * 2 + 1);
    }

    /// The author is an agent carrying the label, never the human.
    #[test]
    fn the_author_is_the_label() {
        let id = AgentId::resolve(Some("lead"), &env_of(&[]), Some("codex"));
        assert_eq!(id.author(), Author::agent("codex@lead"));
        assert!(!id.author().is_human());
    }
}
