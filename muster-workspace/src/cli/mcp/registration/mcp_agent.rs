use strum::{Display, EnumIter, EnumProperty, EnumString};

use crate::cli::mcp::registration::agent_cli::AgentCli;

/// The property naming the agent's binary.
const PROGRAM: &str = "program";
/// The property naming its verb that installs a plugin.
const INSTALL: &str = "install";
/// The property naming its verb that uninstalls one.
const UNINSTALL: &str = "uninstall";
/// The property naming its verb that refreshes a marketplace from its source.
const REFRESH: &str = "refresh";
/// The property naming its verb that moves an installed plugin to the
/// marketplace's version, when installing again does not.
const UPDATE: &str = "update";
/// The property holding what joins a repository and the git ref it is
/// pinned to.
const REF_MARK: &str = "ref_mark";
/// The property holding what to tell the person once the plugin is in.
const AFTER_ADD: &str = "after_add";

/// An agent `muster mcp add` sets muster up with. Each agent with a plugin
/// CLI names its words as properties; Zed has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through its plugin commands.
    #[strum(props(
        program = "claude",
        install = "install",
        uninstall = "uninstall",
        refresh = "update",
        update = "update",
        ref_mark = "#"
    ))]
    Claude,
    /// OpenAI's Codex CLI, through its plugin commands; adding the plugin
    /// again moves it to the marketplace's version.
    #[strum(props(
        program = "codex",
        install = "add",
        uninstall = "remove",
        refresh = "upgrade",
        ref_mark = "@",
        after_add = "Codex runs muster's SessionStart hook only once you trust it: open /hooks in Codex and trust the one running `muster mcp context`"
    ))]
    Codex,
    /// The Zed editor, which has no plugin with hooks, so it gets the server
    /// entry alone.
    Zed,
}

impl McpAgent {
    /// The words the agent's own CLI takes to install muster's plugin; none
    /// for Zed, which has no such CLI. The variant decides which, never
    /// whether its properties are there.
    pub fn cli(self) -> Option<AgentCli> {
        match self {
            Self::Zed => None,
            Self::Claude | Self::Codex => Some(
                AgentCli::builder()
                    .program(self.word(PROGRAM))
                    .install(self.word(INSTALL))
                    .uninstall(self.word(UNINSTALL))
                    .refresh(self.word(REFRESH))
                    .update(self.get_str(UPDATE))
                    .ref_mark(self.word(REF_MARK))
                    .after_add(self.get_str(AFTER_ADD))
                    .build(),
            ),
        }
    }

    /// The property `name`, which every agent with a CLI sets; empty when a
    /// variant left it out, which the tests catch.
    fn word(self, name: &str) -> &'static str {
        self.get_str(name).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// The properties every agent with a plugin CLI sets.
    const REQUIRED: [&str; 5] = [PROGRAM, INSTALL, UNINSTALL, REFRESH, REF_MARK];

    /// Every agent with a plugin CLI sets each required property, so none
    /// builds a command with an empty word.
    #[test]
    fn every_plugin_agent_sets_each_required_property() {
        for agent in McpAgent::iter().filter(|agent| agent.cli().is_some()) {
            for name in REQUIRED {
                assert!(
                    agent.get_str(name).is_some_and(|word| !word.is_empty()),
                    "{agent} sets {name}"
                );
            }
        }
    }

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
