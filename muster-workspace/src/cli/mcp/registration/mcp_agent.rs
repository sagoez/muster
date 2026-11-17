use strum::{Display, EnumIter, EnumProperty, EnumString};

/// The property naming the agent's binary, whose plugin commands install
/// muster's plugin; Zed has none.
pub const PROGRAM: &str = "program";
/// The property naming the agent's verb that installs a plugin.
pub const INSTALL_VERB: &str = "install";
/// The property naming the agent's verb that uninstalls one.
pub const UNINSTALL_VERB: &str = "uninstall";
/// The property holding what to tell the person once the plugin is in,
/// when the agent asks something more of them.
pub const AFTER_ADD: &str = "after_add";

/// An agent `muster mcp add` sets muster up with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through its plugin commands.
    #[strum(props(program = "claude", install = "install", uninstall = "uninstall"))]
    Claude,
    /// OpenAI's Codex CLI, through its plugin commands.
    #[strum(props(
        program = "codex",
        install = "add",
        uninstall = "remove",
        after_add = "Codex runs muster's SessionStart hook once you trust it: open /hooks in Codex and trust it there"
    ))]
    Codex,
    /// The Zed editor, which has no plugin with hooks, so it gets the server
    /// entry alone.
    Zed,
}

#[cfg(test)]
mod tests {
    use strum::{EnumProperty, IntoEnumIterator};

    use super::{AFTER_ADD, INSTALL_VERB, McpAgent, PROGRAM, UNINSTALL_VERB};

    #[test]
    fn only_zed_lacks_a_program_and_every_program_has_both_verbs() {
        for agent in McpAgent::iter() {
            let program = agent.get_str(PROGRAM);
            assert_eq!(program.is_none(), agent == McpAgent::Zed, "{agent}");
            assert_eq!(
                agent.get_str(INSTALL_VERB).is_some(),
                program.is_some(),
                "{agent}"
            );
            assert_eq!(
                agent.get_str(UNINSTALL_VERB).is_some(),
                program.is_some(),
                "{agent}"
            );
        }
        assert_eq!(McpAgent::Claude.get_str(PROGRAM), Some("claude"));
        assert_eq!(McpAgent::Codex.get_str(PROGRAM), Some("codex"));
        assert!(McpAgent::Claude.get_str(AFTER_ADD).is_none());
        assert!(McpAgent::Codex.get_str(AFTER_ADD).is_some());
    }
}
