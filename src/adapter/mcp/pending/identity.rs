use crate::domain::coordination::Author;

/// Environment variable naming the agent.
const AGENT_ENV: &str = "MUSTER_AGENT";
/// Environment variable herdr sets to the pane an agent runs in.
const HERDR_PANE_ENV: &str = "HERDR_PANE_ID";
/// Prefix of an id taken from a herdr pane.
const HERDR_PANE_PREFIX: &str = "herdr-pane-";
/// Prefix of an id made up for one server process.
const SESSION_PREFIX: &str = "session-";
/// Hex digits of a made-up session id.
const SESSION_HEX_DIGITS: usize = 6;
/// Joins the client's name to the id in the label.
const CLIENT_SEPARATOR: char = '@';

/// Who an MCP connection writes as. Stand-in for package C's
/// `domain/coordination/agent_id.rs`, which owns the resolution rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentId(String);

impl AgentId {
    /// The id from, first match wins, `flag` (`--as`), `MUSTER_AGENT`,
    /// herdr's pane, or a random session id; labelled `<client>@<id>` when
    /// the client named itself.
    pub fn resolve(
        flag: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
        client: Option<&str>,
    ) -> Self {
        let present = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
        let id = present(flag.map(str::to_string))
            .or_else(|| present(env(AGENT_ENV)))
            .or_else(|| {
                present(env(HERDR_PANE_ENV))
                    .map(|pane| format!("{HERDR_PANE_PREFIX}{}", pane.trim()))
            })
            .unwrap_or_else(session_id);
        let id = id.trim();
        Self(
            match client.map(str::trim).filter(|client| !client.is_empty()) {
                Some(client) => format!("{client}{CLIENT_SEPARATOR}{id}"),
                None => id.to_string(),
            },
        )
    }

    /// The author coordination writes carry.
    pub fn author(&self) -> Author {
        Author::agent(&self.0)
    }
}

/// A random id for one server process.
fn session_id() -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{SESSION_PREFIX}{}", &random[..SESSION_HEX_DIGITS])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No environment at all.
    fn none(_: &str) -> Option<String> {
        None
    }

    /// The flag beats the environment, which beats herdr's pane; the client
    /// prefixes the label.
    #[test]
    fn the_first_source_wins_and_the_client_prefixes() {
        let env = |key: &str| match key {
            AGENT_ENV => Some("reviewer".to_string()),
            HERDR_PANE_ENV => Some("3".to_string()),
            _ => None,
        };
        assert_eq!(
            AgentId::resolve(Some("builder"), &env, Some("claude-code"))
                .author()
                .label(),
            "claude-code@builder"
        );
        assert_eq!(
            AgentId::resolve(None, &env, None).author().label(),
            "reviewer"
        );
        let pane = |key: &str| (key == HERDR_PANE_ENV).then(|| "3".to_string());
        assert_eq!(
            AgentId::resolve(None, &pane, Some("claude-code"))
                .author()
                .label(),
            "claude-code@herdr-pane-3"
        );
    }

    /// With nothing to go on, the id is a random session id, never the
    /// human.
    #[test]
    fn a_session_id_is_made_up() {
        let label = AgentId::resolve(Some("  "), &none, None).author();
        assert!(label.label().starts_with(SESSION_PREFIX), "{label}");
        assert_eq!(
            label.label().len(),
            SESSION_PREFIX.len() + SESSION_HEX_DIGITS
        );
        assert!(!label.is_human());
    }
}
