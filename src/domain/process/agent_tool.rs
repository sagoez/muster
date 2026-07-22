use strum::EnumString;

use crate::domain::value::CommandLine;

/// Supported coding-agent command presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum AgentTool {
    /// Anthropic Claude Code.
    Claude,
    /// OpenAI Codex CLI.
    Codex,
    /// Google Gemini CLI.
    Gemini,
    /// Sourcegraph Amp.
    Amp,
    /// OpenCode.
    Opencode,
    /// GitHub Copilot CLI.
    Copilot,
    /// Moonshot Kimi CLI.
    Kimi,
    /// A user-supplied agent command.
    Custom,
}

impl AgentTool {
    /// Labels shown in the agent-session launcher.
    pub const OPTIONS: [&str; 8] = [
        "Claude", "Codex", "Gemini", "Amp", "OpenCode", "Copilot", "Kimi", "Custom",
    ];

    /// Returns the human-readable tool name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Amp => "Amp",
            Self::Opencode => "OpenCode",
            Self::Copilot => "Copilot",
            Self::Kimi => "Kimi",
            Self::Custom => "Custom agent",
        }
    }

    /// Returns the preset executable, or `None` when a custom command is needed.
    pub const fn default_command(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::Gemini => Some("gemini"),
            Self::Amp => Some("amp"),
            Self::Opencode => Some("opencode"),
            Self::Copilot => Some("copilot"),
            Self::Kimi => Some("kimi"),
            Self::Custom => None,
        }
    }

    /// Infers a preset from the executable at the start of `command`.
    pub fn from_command(command: Option<&CommandLine>) -> Self {
        let Some(executable) = command
            .and_then(|command| command.as_ref().split_whitespace().next())
            .and_then(|executable| executable.rsplit('/').next())
        else {
            return Self::Custom;
        };

        Self::OPTIONS
            .iter()
            .filter_map(|option| option.parse().ok())
            .find(|tool: &Self| tool.default_command() == Some(executable))
            .unwrap_or(Self::Custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path-qualified known executable still selects its preset.
    #[test]
    fn infers_known_executables_from_commands() {
        let command = CommandLine::try_new("/usr/local/bin/codex --full-auto").unwrap();

        assert_eq!(AgentTool::from_command(Some(&command)), AgentTool::Codex);
    }

    /// An unrecognized executable uses generic agent behavior.
    #[test]
    fn treats_unknown_commands_as_custom() {
        let command = CommandLine::try_new("my-agent").unwrap();

        assert_eq!(AgentTool::from_command(Some(&command)), AgentTool::Custom);
    }

    /// Every choice exposed by the launcher maps back to an agent tool.
    #[test]
    fn every_launcher_option_parses() {
        for option in AgentTool::OPTIONS {
            assert!(option.parse::<AgentTool>().is_ok());
        }
    }
}
