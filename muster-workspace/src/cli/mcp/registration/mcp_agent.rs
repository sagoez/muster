use strum::{Display, EnumIter, EnumProperty, EnumString};

/// The property naming the agent's binary, whose plugin commands install
/// muster's plugin; Zed has none.
pub const PROGRAM: &str = "program";
/// The property naming the agent's verb that installs a plugin.
pub const INSTALL_VERB: &str = "install";
/// The property naming the agent's verb that uninstalls one.
pub const UNINSTALL_VERB: &str = "uninstall";
/// The property holding what joins a marketplace's repository and the git
/// ref it is pinned to in the agent's `marketplace add`.
pub const REF_MARK: &str = "ref_mark";
/// The property holding what to tell the person once the plugin is in,
/// when the agent asks something more of them.
pub const AFTER_ADD: &str = "after_add";

/// An agent `muster mcp add` sets muster up with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through its plugin commands.
    #[strum(props(
        program = "claude",
        install = "install",
        uninstall = "uninstall",
        ref_mark = "#"
    ))]
    Claude,
    /// OpenAI's Codex CLI, through its plugin commands.
    #[strum(props(
        program = "codex",
        install = "add",
        uninstall = "remove",
        ref_mark = "@",
        after_add = "Codex runs muster's SessionStart hook only once you trust it: open /hooks in Codex and trust the one running `muster mcp context`"
    ))]
    Codex,
    /// The Zed editor, which has no plugin with hooks, so it gets the server
    /// entry alone.
    Zed,
}

#[cfg(test)]
mod tests {
    use strum::{EnumProperty, IntoEnumIterator};

    use super::*;

    /// Only Zed lacks a program, and every agent with one names each verb
    /// and ref mark muster passes it.
    #[test]
    fn only_zed_lacks_a_program_and_every_program_has_its_words() {
        for agent in McpAgent::iter() {
            let program = agent.get_str(PROGRAM);
            assert_eq!(program.is_none(), agent == McpAgent::Zed, "{agent}");
            for word in [INSTALL_VERB, UNINSTALL_VERB, REF_MARK] {
                assert_eq!(
                    agent.get_str(word).is_some(),
                    program.is_some(),
                    "{agent} {word}"
                );
            }
        }
        assert_eq!(McpAgent::Claude.get_str(PROGRAM), Some("claude"));
        assert_eq!(McpAgent::Codex.get_str(PROGRAM), Some("codex"));
        assert!(McpAgent::Claude.get_str(AFTER_ADD).is_none());
        assert!(
            McpAgent::Codex
                .get_str(AFTER_ADD)
                .is_some_and(|note| note.contains("muster mcp context")),
            "the trust note names what it trusts"
        );
    }
}
