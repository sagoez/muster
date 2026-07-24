use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString, IntoEnumIterator};

use crate::domain::value::CommandLine;

/// Supported coding-agent command presets.
#[derive(
    AsRefStr,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    /// Anthropic Claude Code.
    #[strum(to_string = "Claude")]
    Claude,
    /// OpenAI Codex CLI.
    #[strum(to_string = "Codex")]
    Codex,
    /// Google Gemini CLI.
    #[strum(to_string = "Gemini")]
    Gemini,
    /// Sourcegraph Amp.
    #[strum(to_string = "Amp")]
    Amp,
    /// OpenCode.
    #[strum(to_string = "OpenCode")]
    Opencode,
    /// GitHub Copilot CLI.
    #[strum(to_string = "Copilot")]
    Copilot,
    /// Moonshot Kimi CLI.
    #[strum(to_string = "Kimi")]
    Kimi,
    /// A user-supplied agent command.
    #[strum(to_string = "Custom agent", serialize = "custom")]
    Custom,
}

impl AgentTool {
    /// Iterates providers in the order used by the launcher.
    pub fn options() -> impl Iterator<Item = Self> {
        Self::iter()
    }

    /// Returns the stable lowercase token used in provider hook payloads.
    pub const fn protocol_token(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Amp => "amp",
            Self::Opencode => "opencode",
            Self::Copilot => "copilot",
            Self::Kimi => "kimi",
            Self::Custom => "custom",
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
            .and_then(|executable| executable.rsplit(['/', '\\']).next())
            .map(|executable| {
                executable
                    .strip_suffix(".exe")
                    .or_else(|| executable.strip_suffix(".cmd"))
                    .or_else(|| executable.strip_suffix(".bat"))
                    .unwrap_or(executable)
            })
        else {
            return Self::Custom;
        };

        Self::iter()
            .find(|tool| {
                tool.default_command().is_some_and(|default_command| {
                    #[cfg(windows)]
                    {
                        executable.eq_ignore_ascii_case(default_command)
                    }
                    #[cfg(not(windows))]
                    {
                        executable == default_command
                    }
                })
            })
            .unwrap_or(Self::Custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{agent_session::NativeSessionId, process::AgentProtocol};

    /// A path-qualified known executable still selects its preset.
    #[test]
    fn infers_known_executables_from_commands() {
        let command = CommandLine::try_new("/usr/local/bin/codex --full-auto").unwrap();

        assert_eq!(AgentTool::from_command(Some(&command)), AgentTool::Codex);
    }

    /// Windows-style paths and command-launcher suffixes identify presets too.
    #[test]
    fn infers_known_windows_executables_from_commands() {
        let command = CommandLine::try_new(r"C:\Tools\claude.cmd").unwrap();

        assert_eq!(AgentTool::from_command(Some(&command)), AgentTool::Claude);
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
        for tool in AgentTool::options() {
            assert_eq!(tool.to_string().parse::<AgentTool>().unwrap(), tool);
        }
    }

    /// Every provider can select its own parseable launcher option even when
    /// its human-facing label is more descriptive.
    #[test]
    fn every_tool_maps_to_its_launcher_option() {
        for tool in AgentTool::options() {
            assert_eq!(tool.to_string().parse::<AgentTool>().unwrap(), tool);
        }
    }

    /// Human-facing provider labels retain their established capitalization.
    #[test]
    fn displays_title_cased_provider_labels() {
        assert_eq!(AgentTool::Claude.to_string(), "Claude");
        assert_eq!(AgentTool::Codex.to_string(), "Codex");
        assert_eq!(AgentTool::Gemini.to_string(), "Gemini");
        assert_eq!(AgentTool::Opencode.to_string(), "OpenCode");
    }

    /// Protocol tokens stay separate from title-cased display labels.
    #[test]
    fn exposes_lowercase_protocol_tokens() {
        assert_eq!(AgentTool::Claude.protocol_token(), "claude");
        assert_eq!(AgentTool::Opencode.protocol_token(), "opencode");
        assert_eq!(AgentTool::Custom.protocol_token(), "custom");
    }

    /// The public protocol's machine token remains accepted for custom providers.
    #[test]
    fn custom_protocol_token_parses() {
        assert_eq!("custom".parse::<AgentTool>().unwrap(), AgentTool::Custom);
    }

    /// Every known provider has an explicit native resume strategy.
    #[test]
    fn builds_provider_resume_commands() {
        let cases = [
            (AgentTool::Claude, "claude --resume abc"),
            (AgentTool::Codex, "codex resume abc"),
            (AgentTool::Gemini, "gemini --resume abc"),
            (AgentTool::Amp, "amp threads continue abc"),
            (AgentTool::Opencode, "opencode --session abc"),
            (AgentTool::Copilot, "copilot --resume abc"),
            (AgentTool::Kimi, "kimi --session abc"),
        ];
        let native_id = NativeSessionId::try_new("abc").unwrap();
        for (tool, expected) in cases {
            let launch = CommandLine::try_new(tool.default_command().unwrap()).unwrap();
            assert_eq!(
                tool.resume_command(&launch, &native_id).unwrap().as_ref(),
                expected
            );
        }
    }
}
