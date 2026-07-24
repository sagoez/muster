use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::AgentTool;
use crate::domain::{
    agent_session::{AgentSession, AgentSessionId, NativeSessionId},
    value::CommandLine,
};

/// Current version of the public agent event protocol.
pub const AGENT_PROTOCOL_VERSION: u8 = 1;
/// Claude options whose following argument is part of the launch invocation.
const CLAUDE_VALUE_OPTIONS: &[&str] = &[
    "--model",
    "--add-dir",
    "--settings",
    "--permission-mode",
    "--fallback-model",
];
/// Codex options whose following argument is part of the launch invocation.
const CODEX_VALUE_OPTIONS: &[&str] = &[
    "-C",
    "--cd",
    "--sandbox",
    "--ask-for-approval",
    "--profile",
    "--model",
    "--config",
    "--add-dir",
    "--output-schema",
];
/// Gemini options whose following argument is part of the launch invocation.
const GEMINI_VALUE_OPTIONS: &[&str] = &["--model"];

/// Terminal evidence Muster should treat as inferred provider activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentActivitySource {
    /// Ordinary visible output indicates work.
    #[default]
    Output,
    /// Terminal-title changes indicate work.
    Title,
}

/// How a provider's native session identity becomes known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentIdentitySource {
    /// Muster can assign its UUID when launching a new provider session.
    Assigned,
    /// A provider lifecycle event reports the identity after launch.
    Reported,
}

/// Internal strategy defining what Muster launches and which signals it reads
/// for one built-in agent provider.
pub(crate) trait AgentProtocol {
    /// Returns the provider's inferred terminal-activity source.
    fn activity_source(&self) -> AgentActivitySource;

    /// Returns how the provider's native session identity is obtained.
    fn identity_source(&self) -> AgentIdentitySource;

    /// Builds the command for a fresh provider conversation.
    fn new_session_command(
        &self,
        command: &CommandLine,
        session_id: &AgentSessionId,
    ) -> Option<CommandLine>;

    /// Builds the command for resuming a provider conversation.
    fn resume_command(
        &self,
        command: &CommandLine,
        native_id: &NativeSessionId,
    ) -> Option<CommandLine>;
}

impl AgentProtocol for AgentTool {
    fn activity_source(&self) -> AgentActivitySource {
        match self {
            Self::Codex | Self::Gemini | Self::Amp => AgentActivitySource::Title,
            Self::Claude | Self::Opencode | Self::Copilot | Self::Kimi | Self::Custom => {
                AgentActivitySource::Output
            },
        }
    }

    fn identity_source(&self) -> AgentIdentitySource {
        match self {
            Self::Claude => AgentIdentitySource::Assigned,
            Self::Codex
            | Self::Gemini
            | Self::Amp
            | Self::Opencode
            | Self::Copilot
            | Self::Kimi
            | Self::Custom => AgentIdentitySource::Reported,
        }
    }

    fn new_session_command(
        &self,
        command: &CommandLine,
        session_id: &AgentSessionId,
    ) -> Option<CommandLine> {
        let suffix = if self.identity_source() == AgentIdentitySource::Assigned {
            Some(format!(
                "--session-id {}",
                AgentSession::quote_for_command_shell(session_id.as_ref())?
            ))
        } else {
            None
        };
        if suffix.is_some() && !self.command_accepts_provider_arguments(command) {
            return None;
        }
        let command = suffix.map_or_else(
            || command.as_ref().to_string(),
            |suffix| format!("{} {suffix}", command.as_ref()),
        );
        CommandLine::try_new(command).ok()
    }

    fn resume_command(
        &self,
        command: &CommandLine,
        native_id: &NativeSessionId,
    ) -> Option<CommandLine> {
        if !self.command_accepts_provider_arguments(command) {
            return None;
        }
        let id = AgentSession::quote_for_command_shell(native_id.as_ref())?;
        let suffix = match self {
            Self::Claude => format!("--resume {id}"),
            Self::Codex => format!("resume {id}"),
            Self::Gemini => format!("--resume {id}"),
            Self::Amp => format!("threads continue {id}"),
            Self::Opencode => format!("--session {id}"),
            Self::Copilot => format!("--resume {id}"),
            Self::Kimi => format!("--session {id}"),
            Self::Custom => return None,
        };
        CommandLine::try_new(format!("{} {suffix}", command.as_ref())).ok()
    }
}

impl AgentTool {
    /// Whether provider arguments can be appended to this provider's command.
    fn command_accepts_provider_arguments(&self, command: &CommandLine) -> bool {
        AgentSession::launch_command_accepts_provider_arguments(command)
            && Self::command_arguments(command.as_ref()).is_some_and(|arguments| {
                let executable_index = arguments
                    .iter()
                    .position(|argument| !argument.contains('='));
                let executable = executable_index
                    .and_then(|index| Self::provider_executable_name(&arguments[index]));
                executable.is_some_and(|executable| Self::same_provider_command(self, executable))
                    && executable_index.is_some_and(|index| {
                        let mut arguments = arguments[index + 1..].iter().peekable();
                        while let Some(argument) = arguments.next() {
                            if argument == "--" {
                                return false;
                            }
                            if !argument.starts_with('-') {
                                return false;
                            }
                            if !argument.contains('=')
                                && arguments
                                    .peek()
                                    .is_some_and(|value| !value.starts_with('-'))
                            {
                                if self.option_takes_value(argument) {
                                    arguments.next();
                                } else {
                                    return false;
                                }
                            }
                        }
                        true
                    })
            })
    }

    /// Splits safe provider overrides while preserving an unquoted Windows path.
    fn command_arguments(command: &str) -> Option<Vec<String>> {
        let first_argument = command.split_whitespace().next()?;
        if first_argument.contains('\\') && !first_argument.contains(['\'', '"']) {
            return Some(command.split_whitespace().map(str::to_string).collect());
        }
        shlex::split(command)
    }

    /// Returns a command path's provider executable name without Windows-only
    /// path and launcher suffixes.
    fn provider_executable_name(command: &str) -> Option<&str> {
        let executable = command.rsplit(['/', '\\']).next()?;
        Some(
            executable
                .strip_suffix(".exe")
                .or_else(|| executable.strip_suffix(".cmd"))
                .or_else(|| executable.strip_suffix(".bat"))
                .unwrap_or(executable),
        )
    }

    /// Compares provider executable names using the host shell's semantics.
    fn same_provider_command(&self, executable: &str) -> bool {
        self.default_command().is_some_and(|default_command| {
            #[cfg(windows)]
            {
                executable.eq_ignore_ascii_case(default_command)
            }
            #[cfg(not(windows))]
            {
                executable == default_command
            }
        })
    }

    /// Returns whether `option` consumes no following positional value.
    fn option_takes_value(self, option: &str) -> bool {
        match self {
            Self::Claude => CLAUDE_VALUE_OPTIONS.contains(&option),
            Self::Codex => CODEX_VALUE_OPTIONS.contains(&option),
            Self::Gemini => GEMINI_VALUE_OPTIONS.contains(&option),
            Self::Amp | Self::Opencode | Self::Copilot | Self::Kimi | Self::Custom => false,
        }
    }
}

/// Event names accepted by the public JSON wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProtocolEventKind {
    /// A provider created or resumed a native conversation.
    SessionStarted,
}

/// Canonical versioned event an agent can send to `muster hook capture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct AgentProtocolEvent {
    /// Protocol schema version, currently [`AGENT_PROTOCOL_VERSION`].
    version: u8,
    /// Lifecycle event represented by this payload.
    event: AgentProtocolEventKind,
    /// Provider-owned identity used by its native resume command.
    session_id: NativeSessionId,
}

impl AgentProtocolEvent {
    /// Creates the canonical event for a provider conversation becoming active.
    pub fn session_started(session_id: NativeSessionId) -> Self {
        Self::builder()
            .version(AGENT_PROTOCOL_VERSION)
            .event(AgentProtocolEventKind::SessionStarted)
            .session_id(session_id)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The canonical constructor fixes the public wire version and event name.
    #[test]
    fn session_started_serializes_the_versioned_wire_contract() {
        let event = AgentProtocolEvent::session_started(
            NativeSessionId::try_new("provider-session").unwrap(),
        );

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "version": AGENT_PROTOCOL_VERSION,
                "event": "session_started",
                "session_id": "provider-session"
            })
        );
    }

    /// Provider implementations can be consumed behind the public protocol
    /// trait rather than requiring an enum-specific call site.
    #[test]
    fn protocol_is_object_safe() {
        let protocol: &dyn AgentProtocol = &AgentTool::Codex;

        assert_eq!(protocol.activity_source(), AgentActivitySource::Title);
    }

    /// Only providers that accept caller-assigned IDs receive a session flag;
    /// Copilot reports its identity later through its lifecycle hook.
    #[test]
    fn new_session_commands_respect_provider_identity_ownership() {
        let session_id = AgentSessionId::generate().unwrap();
        let claude = CommandLine::try_new("claude").unwrap();
        let copilot = CommandLine::try_new("copilot").unwrap();

        assert_eq!(
            AgentTool::Claude
                .new_session_command(&claude, &session_id)
                .unwrap()
                .as_ref(),
            format!("claude --session-id {session_id}")
        );
        assert_eq!(
            AgentTool::Copilot
                .new_session_command(&copilot, &session_id)
                .unwrap()
                .as_ref(),
            "copilot"
        );
    }

    /// Composed shell commands require an explicit resume template rather than
    /// receiving provider flags after an unrelated final command.
    #[test]
    fn provider_commands_reject_shell_compositions() {
        let command = CommandLine::try_new("codex | tee agent.log").unwrap();
        let wrapper = CommandLine::try_new("bash -lc 'codex'").unwrap();
        let prompt = CommandLine::try_new("codex 'fix auth'").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .is_none()
        );
        assert!(
            AgentTool::Codex
                .resume_command(&wrapper, &native_id)
                .is_none()
        );
        assert!(
            AgentTool::Codex
                .resume_command(&prompt, &native_id)
                .is_none()
        );
        assert!(
            AgentTool::Claude
                .new_session_command(&wrapper, &AgentSessionId::generate().unwrap())
                .is_none()
        );
    }

    /// The end-of-options marker turns every later token positional, so provider
    /// session controls must never be appended after it.
    #[test]
    fn provider_commands_reject_end_of_options_markers() {
        let command = CommandLine::try_new("claude --").unwrap();
        let session_id = AgentSessionId::generate().unwrap();

        assert!(
            AgentTool::Claude
                .new_session_command(&command, &session_id)
                .is_none()
        );
    }

    /// Windows-style executable paths and launcher suffixes remain valid preset
    /// overrides even when the host compiling this test is Unix.
    #[test]
    fn provider_commands_accept_windows_executable_paths() {
        let command = CommandLine::try_new(r"C:\Tools\codex.exe --profile work").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert_eq!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .unwrap()
                .as_ref(),
            r"C:\Tools\codex.exe --profile work resume thread-id"
        );
    }

    /// Preset flags may carry separate values without becoming a shell composition.
    #[test]
    fn provider_commands_accept_option_values() {
        let command = CommandLine::try_new("claude --model opus").unwrap();
        let session_id = AgentSessionId::generate().unwrap();

        assert_eq!(
            AgentTool::Claude
                .new_session_command(&command, &session_id)
                .unwrap()
                .as_ref(),
            format!("claude --model opus --session-id {session_id}")
        );
    }

    /// Codex launch-directory and policy overrides consume their following
    /// values, so a session command remains safe to append.
    #[test]
    fn codex_commands_accept_common_value_options() {
        let command = CommandLine::try_new(
            "codex -C /repo --sandbox workspace-write --ask-for-approval on-request",
        )
        .unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert_eq!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .unwrap()
                .as_ref(),
            "codex -C /repo --sandbox workspace-write --ask-for-approval on-request resume thread-id"
        );
    }

    /// A positional prompt after a boolean option makes provider argument
    /// placement ambiguous and therefore requires an explicit resume template.
    #[test]
    fn provider_commands_reject_prompts_after_boolean_options() {
        let command = CommandLine::try_new("codex --full-auto 'fix auth'").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .is_none()
        );
    }

    /// An unrecognised option cannot safely consume a prompt before appended
    /// provider arguments.
    #[test]
    fn provider_commands_reject_prompts_after_unlisted_options() {
        let command = CommandLine::try_new("codex --search 'fix bug'").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .is_none()
        );
    }
}
