use strum::{Display, EnumIter, EnumMessage, EnumProperty, EnumString};

/// The property naming the binary whose `mcp add` and `mcp remove` register
/// servers; Zed has none.
pub const PROGRAM: &str = "program";

/// An agent `muster mcp add` registers muster's MCP server with. Its message
/// is its name as a person knows it.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter, EnumMessage, EnumProperty,
)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through `claude mcp`.
    #[strum(message = "Claude Code", props(program = "claude"))]
    Claude,
    /// OpenAI's Codex CLI, through `codex mcp`.
    #[strum(message = "Codex", props(program = "codex"))]
    Codex,
    /// The Zed editor, which has no such command.
    #[strum(message = "Zed")]
    Zed,
}

#[cfg(test)]
mod tests {
    use strum::{EnumMessage, EnumProperty, IntoEnumIterator};

    use super::{McpAgent, PROGRAM};

    #[test]
    fn every_agent_has_a_title_and_only_zed_lacks_a_program() {
        for agent in McpAgent::iter() {
            assert!(agent.get_message().is_some(), "{agent}");
            assert_eq!(
                agent.get_str(PROGRAM).is_none(),
                agent == McpAgent::Zed,
                "{agent}"
            );
        }
        assert_eq!(McpAgent::Claude.get_str(PROGRAM), Some("claude"));
        assert_eq!(McpAgent::Codex.get_str(PROGRAM), Some("codex"));
    }
}
