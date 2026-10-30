use serde::Serialize;
use strum::{Display, EnumIter, EnumString};

use super::mcp_scope::McpScope;

/// Claude Code, as a person knows it.
const CLAUDE_TITLE: &str = "Claude Code";
/// Codex, as a person knows it.
const CODEX_TITLE: &str = "Codex";
/// Zed, as a person knows it.
const ZED_TITLE: &str = "Zed";
/// Claude Code's binary.
const CLAUDE_BINARIES: &[&str] = &["claude"];
/// Codex's binary.
const CODEX_BINARIES: &[&str] = &["codex"];
/// Zed's binary, upstream and as distributions that rename it ship it.
const ZED_BINARIES: &[&str] = &["zed", "zeditor"];

/// An agent whose config muster's MCP server can be added to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    /// Claude Code.
    Claude,
    /// OpenAI's Codex CLI.
    Codex,
    /// The Zed editor's agent panel.
    Zed,
}

impl Agent {
    /// The agent's name as a person knows it.
    pub fn title(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_TITLE,
            Self::Codex => CODEX_TITLE,
            Self::Zed => ZED_TITLE,
        }
    }

    /// Whether the agent reads MCP servers from `scope`.
    pub fn supports(self, scope: McpScope) -> bool {
        match self {
            Self::Claude => true,
            Self::Codex => matches!(scope, McpScope::User | McpScope::Project),
            Self::Zed => scope == McpScope::User,
        }
    }

    /// The names the agent's own binary goes by on `PATH`.
    pub fn binaries(self) -> &'static [&'static str] {
        match self {
            Self::Claude => CLAUDE_BINARIES,
            Self::Codex => CODEX_BINARIES,
            Self::Zed => ZED_BINARIES,
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::adapter::agent_config::DEFAULT_MCP_SCOPE;

    /// Every agent supports the default scope, and only Claude Code has a
    /// local scope and only Zed lacks a project one.
    #[test]
    fn scopes_follow_each_agents_docs() {
        for agent in Agent::iter() {
            assert!(agent.supports(DEFAULT_MCP_SCOPE), "{agent}");
        }
        assert!(Agent::Claude.supports(McpScope::Project));
        assert!(Agent::Codex.supports(McpScope::Project));
        assert!(!Agent::Codex.supports(McpScope::Local));
        assert!(!Agent::Zed.supports(McpScope::Project));
        assert!(!Agent::Zed.supports(McpScope::Local));
        assert_eq!("claude".parse::<Agent>().unwrap(), Agent::Claude);
    }
}
