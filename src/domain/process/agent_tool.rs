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

    /// Infers a preset from the executable in `command`, skipping any leading
    /// `NAME=VALUE` environment assignments so a prefixed provider (for example
    /// `ANTHROPIC_MODEL=opus claude`) is still recognized rather than treated as
    /// a custom command, which would reject its provider's lifecycle reports.
    pub fn from_command(command: Option<&CommandLine>) -> Self {
        let Some(command) = command else {
            return Self::Custom;
        };
        let tokens = Self::tokenize(command.as_ref());
        let Some(executable) = tokens
            .iter()
            .map(String::as_str)
            .find(|token| !is_env_assignment(token))
            .map(Self::executable_head)
            .and_then(|executable| executable.rsplit(['/', '\\']).next())
            .map(strip_launcher_suffix)
        else {
            return Self::Custom;
        };

        Self::identify(executable)
    }

    /// Splits a command into tokens with POSIX shell quoting and escaping, so a
    /// quoted or backslash-escaped assignment value (`MODEL="two words"` or
    /// `MODEL=two\ words`) stays one token before the executable. Falls back to
    /// whitespace splitting on a command shlex cannot parse (an unbalanced quote).
    #[cfg(not(windows))]
    fn tokenize(command: &str) -> Vec<String> {
        shlex::split(command)
            .unwrap_or_else(|| command.split_whitespace().map(str::to_string).collect())
    }

    /// Splits a command on Windows, where backslashes are literal path separators
    /// (`C:\Tools\claude.exe`) rather than escapes. Honors double and single quotes
    /// so a quoted assignment value stays one token; Windows has no POSIX
    /// backslash escaping. Not a full shell parser, only enough to isolate the
    /// executable behind any leading `NAME=VALUE` prefixes.
    #[cfg(windows)]
    fn tokenize(command: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut has_token = false;
        let mut in_single = false;
        let mut in_double = false;
        for character in command.chars() {
            if character == '\'' && !in_double {
                in_single = !in_single;
                has_token = true;
            } else if character == '"' && !in_single {
                in_double = !in_double;
                has_token = true;
            } else if character.is_whitespace() && !in_single && !in_double {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            } else {
                current.push(character);
                has_token = true;
            }
        }
        if has_token {
            tokens.push(current);
        }
        tokens
    }

    /// The executable portion of a token, cut at the first shell operator so a
    /// compact composition without surrounding spaces (`claude|tee`, `claude;echo`)
    /// does not fold the operator into the executable name and misclassify the
    /// provider as `Custom`. `shlex` splits words but not operators, so it leaves
    /// `claude|tee` as one token. Only unambiguous operators are cut, so a path such
    /// as `Program Files (x86)` keeps its parentheses.
    fn executable_head(token: &str) -> &str {
        token
            .find(['|', '&', ';', '<', '>'])
            .map_or(token, |index| &token[..index])
    }

    /// Matches a bare executable name against each preset's default command.
    fn identify(executable: &str) -> Self {
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

/// Windows launcher suffixes stripped from an executable name so a `.exe`/`.cmd`/
/// `.bat` shim resolves to the bare provider name.
const LAUNCHER_SUFFIXES: [&str; 3] = [".exe", ".cmd", ".bat"];

/// Strips a [`LAUNCHER_SUFFIXES`] suffix from `name`, case-insensitively on Windows
/// where file names and extensions ignore case (so `CLAUDE.EXE` reduces to
/// `CLAUDE` and still matches a provider), and case-sensitively elsewhere where
/// those suffixes are only launcher shims. At most one suffix is removed. Shared
/// with [`super::agent_protocol`] so command inference and argument injection agree
/// on the bare executable name.
pub(super) fn strip_launcher_suffix(name: &str) -> &str {
    #[cfg(windows)]
    let lowered = name.to_ascii_lowercase();
    for suffix in LAUNCHER_SUFFIXES {
        #[cfg(windows)]
        let stripped = lowered.strip_suffix(suffix).map(|head| &name[..head.len()]);
        #[cfg(not(windows))]
        let stripped = name.strip_suffix(suffix);
        if let Some(stripped) = stripped {
            return stripped;
        }
    }
    name
}

/// Whether a shell token is a leading `NAME=VALUE` environment assignment, which
/// precedes the executable rather than naming it. The name must be a valid shell
/// identifier so a flag such as `--model=opus` is not mistaken for one. Shared with
/// [`super::agent_protocol`] so command inference and argument injection agree on
/// where the executable begins.
pub(super) fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
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
    /// Only on Windows, where backslashes are literal path separators; under POSIX
    /// tokenization they are escapes, so this is not a valid Unix command.
    #[cfg(windows)]
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

    /// Leading environment assignments do not hide the provider, so a prefixed
    /// command still resumes and captures under its real preset.
    #[test]
    fn infers_the_provider_behind_environment_assignments() {
        let prefixed = CommandLine::try_new("ANTHROPIC_MODEL=opus claude").unwrap();
        assert_eq!(AgentTool::from_command(Some(&prefixed)), AgentTool::Claude);

        let multiple = CommandLine::try_new("FOO=1 BAR=2 codex --full-auto").unwrap();
        assert_eq!(AgentTool::from_command(Some(&multiple)), AgentTool::Codex);
    }

    /// A double-quoted assignment value with spaces stays one token on either
    /// platform, so the provider after it is still recognized rather than split
    /// into a bogus executable.
    #[test]
    fn infers_the_provider_behind_a_quoted_assignment_value() {
        let double = CommandLine::try_new(r#"MODEL="two words" claude"#).unwrap();
        assert_eq!(AgentTool::from_command(Some(&double)), AgentTool::Claude);
    }

    /// On Unix, POSIX single quotes and backslash escapes keep an assignment
    /// value with spaces together, so the provider behind it is still recognized
    /// and its lifecycle reports are not rejected as a mismatch.
    #[cfg(unix)]
    #[test]
    fn infers_the_provider_behind_posix_escaped_assignments() {
        let escaped = CommandLine::try_new(r"MODEL=two\ words claude").unwrap();
        assert_eq!(AgentTool::from_command(Some(&escaped)), AgentTool::Claude);

        let single = CommandLine::try_new("MODEL='two words' codex").unwrap();
        assert_eq!(AgentTool::from_command(Some(&single)), AgentTool::Codex);
    }

    /// A `--flag=value` argument is not an environment assignment, so it never
    /// causes the executable itself to be skipped.
    #[test]
    fn does_not_treat_flags_as_environment_assignments() {
        let command = CommandLine::try_new("claude --model=opus").unwrap();

        assert_eq!(AgentTool::from_command(Some(&command)), AgentTool::Claude);
    }

    /// A lowercase launcher suffix reduces to the bare name on every platform.
    #[test]
    fn strips_a_lowercase_launcher_suffix() {
        assert_eq!(strip_launcher_suffix("claude.exe"), "claude");
        assert_eq!(strip_launcher_suffix("codex.cmd"), "codex");
        assert_eq!(strip_launcher_suffix("gemini.bat"), "gemini");
        assert_eq!(strip_launcher_suffix("plain"), "plain");
    }

    /// On Windows, executable names and their launcher suffixes are
    /// case-insensitive, so an upper-case `CLAUDE.EXE` or a mixed-case path still
    /// resolves to its provider rather than falling back to Custom and losing
    /// identity capture and durable resume.
    #[cfg(windows)]
    #[test]
    fn infers_a_windows_uppercase_launcher_name() {
        for (command, expected) in [
            ("CLAUDE.EXE", AgentTool::Claude),
            ("CODEX.CMD", AgentTool::Codex),
            (r"C:\Tools\Claude.Exe --model opus", AgentTool::Claude),
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert_eq!(AgentTool::from_command(Some(&parsed)), expected);
        }
    }

    /// A shell operator abutting the executable with no surrounding spaces is not
    /// part of its name; the provider is still inferred so its lifecycle reports are
    /// captured and native resume is preserved.
    #[test]
    fn infers_the_provider_before_a_compact_shell_operator() {
        for command in [
            "claude|tee agent.log",
            "claude;echo done",
            "codex&",
            "codex>out.log",
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert_ne!(
                AgentTool::from_command(Some(&parsed)),
                AgentTool::Custom,
                "`{command}` must infer its provider, not fall back to Custom"
            );
        }
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
