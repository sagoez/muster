use strum::{Display, EnumIter, EnumString};

/// Claude Code's binary.
const CLAUDE_PROGRAM: &str = "claude";
/// Codex's binary.
const CODEX_PROGRAM: &str = "codex";
/// Codex, as a person knows it.
const CODEX_TITLE: &str = "Codex";
/// Claude Code, as a person knows it.
const CLAUDE_TITLE: &str = "Claude Code";
/// Zed, as a person knows it.
const ZED_TITLE: &str = "Zed";

/// An agent `muster mcp add` registers muster's MCP server with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum McpAgent {
    /// Claude Code, through `claude mcp`.
    Claude,
    /// OpenAI's Codex CLI, through `codex mcp`.
    Codex,
    /// The Zed editor, which has no such command.
    Zed,
}

impl McpAgent {
    /// The agent's name as a person knows it.
    pub fn title(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_TITLE,
            Self::Codex => CODEX_TITLE,
            Self::Zed => ZED_TITLE,
        }
    }

    /// The binary whose `mcp add` and `mcp remove` register servers, none
    /// for Zed.
    pub fn program(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some(CLAUDE_PROGRAM),
            Self::Codex => Some(CODEX_PROGRAM),
            Self::Zed => None,
        }
    }
}
