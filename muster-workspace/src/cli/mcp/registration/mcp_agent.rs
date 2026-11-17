use strum::{Display, EnumIter, EnumString};

use crate::cli::mcp::registration::agent_cli::AgentCli;

/// Claude Code's binary.
const CLAUDE: &str = "claude";
/// Codex's binary.
const CODEX: &str = "codex";
/// Claude Code's verb that installs a plugin.
const CLAUDE_INSTALL: &str = "install";
/// Claude Code's verb that uninstalls one.
const CLAUDE_UNINSTALL: &str = "uninstall";
/// Claude Code's verb that refreshes a marketplace, and that moves an
/// installed plugin to the marketplace's version.
const CLAUDE_UPDATE: &str = "update";
/// Codex's verb that installs a plugin, which also moves an installed one
/// to the marketplace's version.
const CODEX_INSTALL: &str = "add";
/// Codex's verb that uninstalls one.
const CODEX_UNINSTALL: &str = "remove";
/// Codex's verb that refreshes a marketplace.
const CODEX_REFRESH: &str = "upgrade";
/// How Claude Code pins a marketplace's repository to a git ref.
const CLAUDE_REF_MARK: &str = "#";
/// How Codex pins one.
const CODEX_REF_MARK: &str = "@";
/// What Codex asks of the person before muster's hook runs.
const CODEX_TRUST_NOTE: &str = "Codex runs muster's SessionStart hook only once you trust it: open /hooks in Codex and trust the one running `muster mcp context`";

/// An agent `muster mcp add` sets muster up with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through its plugin commands.
    Claude,
    /// OpenAI's Codex CLI, through its plugin commands.
    Codex,
    /// The Zed editor, which has no plugin with hooks, so it gets the server
    /// entry alone.
    Zed,
}

impl McpAgent {
    /// The words the agent's own CLI takes to install muster's plugin; none
    /// for Zed, which has no such CLI.
    pub fn cli(self) -> Option<AgentCli> {
        match self {
            Self::Claude => Some(
                AgentCli::builder()
                    .program(CLAUDE)
                    .install(CLAUDE_INSTALL)
                    .uninstall(CLAUDE_UNINSTALL)
                    .refresh(CLAUDE_UPDATE)
                    .update(Some(CLAUDE_UPDATE))
                    .ref_mark(CLAUDE_REF_MARK)
                    .after_add(None)
                    .build(),
            ),
            Self::Codex => Some(
                AgentCli::builder()
                    .program(CODEX)
                    .install(CODEX_INSTALL)
                    .uninstall(CODEX_UNINSTALL)
                    .refresh(CODEX_REFRESH)
                    .update(None)
                    .ref_mark(CODEX_REF_MARK)
                    .after_add(Some(CODEX_TRUST_NOTE))
                    .build(),
            ),
            Self::Zed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Only Zed lacks a CLI, each agent refreshes its marketplace with its
    /// own verb, Claude Code alone updates the plugin after installing it,
    /// and Codex alone asks the person to trust the hook, naming what it
    /// runs.
    #[test]
    fn only_zed_lacks_a_cli_and_codex_names_the_hook_to_trust() {
        for agent in McpAgent::iter() {
            assert_eq!(agent.cli().is_none(), agent == McpAgent::Zed, "{agent}");
        }
        let (claude, codex) = (
            McpAgent::Claude.cli().unwrap(),
            McpAgent::Codex.cli().unwrap(),
        );
        assert_eq!(
            (claude.program(), claude.install(), claude.uninstall()),
            ("claude", "install", "uninstall")
        );
        assert_eq!(
            (codex.program(), codex.install(), codex.uninstall()),
            ("codex", "add", "remove")
        );
        assert_eq!((claude.refresh(), codex.refresh()), ("update", "upgrade"));
        assert_eq!((claude.update(), codex.update()), (Some("update"), None));
        assert_eq!((claude.ref_mark(), codex.ref_mark()), ("#", "@"));
        assert!(claude.after_add().is_none());
        assert!(
            codex
                .after_add()
                .is_some_and(|note| note.contains("muster mcp context")),
            "the trust note names what it trusts"
        );
    }
}
