use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{
    AgentTool,
    agent_tool::{is_env_assignment, strip_launcher_suffix},
};
use crate::domain::{
    agent_session::{AgentSession, AgentSessionId, NativeSessionId},
    value::CommandLine,
};

/// Current version of the public agent event protocol.
pub const AGENT_PROTOCOL_VERSION: u8 = 1;

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
        if self.identity_source() != AgentIdentitySource::Assigned {
            // Reported providers capture their identity through hooks, so nothing
            // is injected on the command line.
            return Some(command.clone());
        }
        let args = format!(
            "--session-id {}",
            AgentSession::quote_for_command_shell(session_id.as_ref())?
        );
        self.inject_provider_args(command, &args)
    }

    fn resume_command(
        &self,
        command: &CommandLine,
        native_id: &NativeSessionId,
    ) -> Option<CommandLine> {
        let id = AgentSession::quote_for_command_shell(native_id.as_ref())?;
        // Providers whose resume form is a subcommand rather than a flag.
        let (args, subcommand_based) = match self {
            Self::Claude => (format!("--resume {id}"), false),
            Self::Codex => (format!("resume {id}"), true),
            Self::Gemini => (format!("--resume {id}"), false),
            Self::Amp => (format!("threads continue {id}"), true),
            Self::Opencode => (format!("--session {id}"), false),
            Self::Copilot => (format!("--resume {id}"), false),
            Self::Kimi => (format!("--session {id}"), false),
            Self::Custom => return None,
        };
        // A subcommand-based resume cannot be injected into a command that already
        // selects a subcommand (`codex exec "task"`): the inserted `resume` would
        // turn the original invocation into the resumed session's arguments. Such a
        // session needs an explicit resume template.
        if subcommand_based && command_selects_a_subcommand(command.as_ref()) {
            return None;
        }
        self.inject_provider_args(command, &args)
    }
}

impl AgentTool {
    /// Inserts `args` immediately after the provider executable, so provider
    /// arguments reach the provider before any positional or shell composition and
    /// are never captured by them. Returns `None` for a wrapper command
    /// (`bash -lc 'claude'`, `env -u DEBUG claude`, `mise exec -- claude`) whose
    /// executable is not this provider: the arguments cannot be injected into the
    /// nested provider safely, so such a session needs an explicit resume template.
    fn inject_provider_args(&self, command: &CommandLine, args: &str) -> Option<CommandLine> {
        let command = command.as_ref();
        let (offset, executable) = provider_executable(command)?;
        if !self.executable_is_this_provider(&executable) {
            return None;
        }
        CommandLine::try_new(format!(
            "{} {args}{}",
            &command[..offset],
            &command[offset..]
        ))
        .ok()
    }

    /// Whether `executable` (a bare command name) names this provider, using the
    /// host shell's case sensitivity.
    fn executable_is_this_provider(&self, executable: &str) -> bool {
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
}

/// Finds the provider executable token: the first token that is not a leading
/// `NAME=VALUE` environment assignment. Quote-aware, so a quoted assignment value
/// or executable path is not split at its internal whitespace. Returns the byte
/// offset immediately after that token (where provider arguments are inserted) and
/// the executable's bare name (path segments and Windows launcher suffixes
/// removed). `None` if the command has no executable token.
fn provider_executable(command: &str) -> Option<(usize, String)> {
    let mut cursor = 0;
    loop {
        let token_start = command[cursor..]
            .find(|character: char| !character.is_whitespace())
            .map(|offset| cursor + offset)?;
        let end = token_end(command, token_start);
        let token = &command[token_start..end];
        if is_env_assignment(token) {
            cursor = end;
            continue;
        }
        return Some((end, executable_name(token)));
    }
}

/// Whether the command already selects a subcommand after the provider executable,
/// which resume must not be injected ahead of. Scans the tokens past the
/// executable: a flag (`-x`) is an option, and a spaced option (`--opt`, not the
/// self-contained `--opt=value`) may consume the next bare word as its value; the
/// first positional that is not so consumed decides. A quoted or spaced positional
/// is a prompt, not a subcommand (see [`is_prompt_token`]), so `codex "fix auth"`
/// still resumes; a bare identifier positional is the subcommand. A shell
/// composition (pipe, sequence, redirect) or a trailing comment (`#`) is inserted
/// before rather than into, so it ends the scan without a subcommand.
///
/// Option arity is unknowable without a per-CLI flag table, which muster
/// deliberately does not keep, so a spaced option is assumed to take one value.
/// The only shape this misjudges is a boolean option placed immediately before a
/// lone subcommand (`codex --oss exec`), which then falls through to injection; a
/// subcommand preceded by any value option or any positional is detected.
fn command_selects_a_subcommand(command: &str) -> bool {
    let Some((offset, _)) = provider_executable(command) else {
        return false;
    };
    let mut cursor = offset;
    let mut prior_option_wants_value = false;
    loop {
        let Some(token_start) = command[cursor..]
            .find(|character: char| !character.is_whitespace())
            .map(|position| cursor + position)
        else {
            return false;
        };
        let Some(first) = command[token_start..].chars().next() else {
            return false;
        };
        if matches!(first, '|' | '&' | ';' | '<' | '>' | '(' | '`' | '#') {
            return false;
        }
        let end = token_end(command, token_start);
        let token = &command[token_start..end];
        cursor = end;
        if first == '-' {
            prior_option_wants_value = !token.contains('=');
            continue;
        }
        if prior_option_wants_value {
            prior_option_wants_value = false;
            continue;
        }
        return !is_prompt_token(token);
    }
}

/// Whether a positional token is a prompt rather than a subcommand. A subcommand is
/// a bare identifier (`exec`); a prompt is quoted or contains whitespace (`"fix
/// auth"`, `fix\ auth`), which a bare subcommand never is. This is the only
/// table-free signal that separates the two, since a subcommand's name is
/// provider-specific and muster keeps no per-CLI table. A rare unquoted single-word
/// prompt (`codex hello`) is indistinguishable from a subcommand and treated as
/// one; quoting it resolves the ambiguity.
fn is_prompt_token(token: &str) -> bool {
    token.contains(['\'', '"']) || token.chars().any(char::is_whitespace)
}

/// Whether `character` is an unquoted shell operator that ends the executable
/// token. A compact composition such as `claude|tee` or `codex>out.log` attaches
/// the operator to the executable with no separating whitespace, and the operator
/// is never part of an executable name, so the token ends there. The set matches
/// [`AgentTool::executable_head`] so command inference and argument injection agree
/// on the executable boundary; parentheses, backticks, and `#` are excluded so a
/// quoted path keeps them (none abut the executable unquoted).
fn is_shell_operator(character: char) -> bool {
    matches!(character, '|' | '&' | ';' | '<' | '>')
}

/// The byte offset where the POSIX shell token starting at `start` ends: the next
/// unquoted, unescaped whitespace or shell operator (see [`is_shell_operator`]), or
/// the end of the command. Single- and double-quoted spans and escaped characters
/// keep their whitespace and operators inside the token, matching the `shlex`
/// tokenization `from_command` uses, so an escaped space such as `two\ words` is
/// not split.
#[cfg(not(windows))]
fn token_end(command: &str, start: usize) -> usize {
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = command[start..].char_indices();
    while let Some((offset, character)) = chars.next() {
        if character == '\\' && !in_single {
            // Outside single quotes a backslash escapes the next character, which
            // is then literal and part of this token (an escaped space, quote, ...).
            chars.next();
            continue;
        }
        if !in_single && !in_double && (character.is_whitespace() || is_shell_operator(character)) {
            return start + offset;
        }
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {},
        }
    }
    command.len()
}

/// The byte offset where the token starting at `start` ends on Windows, where a
/// backslash is a literal path separator rather than an escape; only quoted spans
/// keep their internal whitespace and shell operators (see [`is_shell_operator`])
/// inside the token.
#[cfg(windows)]
fn token_end(command: &str, start: usize) -> usize {
    let mut in_single = false;
    let mut in_double = false;
    for (offset, character) in command[start..].char_indices() {
        if !in_single && !in_double && (character.is_whitespace() || is_shell_operator(character)) {
            return start + offset;
        }
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {},
        }
    }
    command.len()
}

/// The bare executable name of a shell token: quotes removed, then the last path
/// segment with any Windows launcher suffix stripped (case-insensitively on
/// Windows via [`strip_launcher_suffix`]).
fn executable_name(token: &str) -> String {
    let unquoted: String = token
        .chars()
        .filter(|&character| character != '"' && character != '\'')
        .collect();
    let base = unquoted.rsplit(['/', '\\']).next().unwrap_or(&unquoted);
    strip_launcher_suffix(base).to_string()
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

    /// Provider arguments are inserted right after the executable, so they reach
    /// the provider (not a downstream command) and are never captured by a trailing
    /// positional or pipe. This is why muster needs no per-CLI flag table.
    #[test]
    fn provider_arguments_are_prepended_after_the_executable() {
        let native_id = NativeSessionId::try_new("thread-id").unwrap();
        let session_id = AgentSessionId::try_new("assigned").unwrap();

        // Before a pipe, so the resume subcommand goes to codex, not tee.
        let piped = CommandLine::try_new("codex | tee agent.log").unwrap();
        assert_eq!(
            AgentTool::Codex
                .resume_command(&piped, &native_id)
                .unwrap()
                .as_ref(),
            "codex resume thread-id | tee agent.log"
        );
        // A flag provider prepends its flag before a positional prompt, so the
        // prompt stays a positional rather than becoming the flag's value.
        let prompt = CommandLine::try_new("claude 'fix bug'").unwrap();
        assert_eq!(
            AgentTool::Claude
                .resume_command(&prompt, &native_id)
                .unwrap()
                .as_ref(),
            "claude --resume thread-id 'fix bug'"
        );
        // An assigned-id new session inserts its flag before the user's flags.
        let claude = CommandLine::try_new("claude --model opus").unwrap();
        assert_eq!(
            AgentTool::Claude
                .new_session_command(&claude, &session_id)
                .unwrap()
                .as_ref(),
            "claude --session-id assigned --model opus"
        );
    }

    /// A compact composition without whitespace around the operator ends the
    /// executable token at the operator, so provider arguments are injected after
    /// the provider (not folded into a `provider|tee` executable that then rejects
    /// injection and leaves the pane with no command).
    #[test]
    fn compact_shell_operators_do_not_break_injection() {
        let session_id = AgentSessionId::try_new("assigned").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        let piped = CommandLine::try_new("claude|tee agent.log").unwrap();
        assert_eq!(
            AgentTool::Claude
                .new_session_command(&piped, &session_id)
                .unwrap()
                .as_ref(),
            "claude --session-id assigned|tee agent.log"
        );

        let redirected = CommandLine::try_new("codex>out.log").unwrap();
        assert_eq!(
            AgentTool::Codex
                .resume_command(&redirected, &native_id)
                .unwrap()
                .as_ref(),
            "codex resume thread-id>out.log"
        );
    }

    /// The executable is found past any leading environment assignments, so
    /// provider arguments land after the provider, not before it.
    #[test]
    fn provider_arguments_skip_environment_prefixes() {
        let command = CommandLine::try_new("MODEL=opus codex --profile work").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert_eq!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .unwrap()
                .as_ref(),
            "MODEL=opus codex resume thread-id --profile work"
        );
    }

    /// A Windows-style executable path resolves the executable to insert after,
    /// even when the host compiling this test is Unix; backslashes stay literal.
    #[test]
    fn provider_arguments_prepend_after_a_windows_executable_path() {
        let command = CommandLine::try_new(r"C:\Tools\codex.exe --profile work").unwrap();
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        assert_eq!(
            AgentTool::Codex
                .resume_command(&command, &native_id)
                .unwrap()
                .as_ref(),
            r"C:\Tools\codex.exe resume thread-id --profile work"
        );
    }

    /// A command that invokes the provider directly resolves to a resume command
    /// regardless of shape - a value option, an unrecognized option, a prompt, or a
    /// composition - because inserting after the executable is always safe, so
    /// muster never rejects such a command over a flag it does not track.
    #[test]
    fn direct_provider_commands_are_always_resumable() {
        let native_id = NativeSessionId::try_new("thread-id").unwrap();
        for command in [
            "claude --system-prompt foo",
            "codex --search 'fix bug'",
            "codex --full-auto 'fix auth'",
            "claude | tee log",
            "gemini 'a prompt'",
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert!(
                AgentTool::from_command(Some(&parsed))
                    .resume_command(&parsed, &native_id)
                    .is_some(),
                "`{command}` must resolve to a resume command"
            );
        }
    }

    /// A wrapper command does not name the provider as its executable, so provider
    /// arguments cannot be injected safely: both a session id and a resume flag are
    /// refused, leaving the session to an explicit resume template.
    #[test]
    fn wrapper_commands_reject_provider_argument_injection() {
        let native_id = NativeSessionId::try_new("thread-id").unwrap();
        let session_id = AgentSessionId::try_new("assigned").unwrap();
        for command in [
            "bash -lc 'claude'",
            "env -u DEBUG claude",
            "mise exec -- claude",
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert!(
                AgentTool::Claude
                    .resume_command(&parsed, &native_id)
                    .is_none(),
                "`{command}` must not inject a resume flag into a wrapper"
            );
            assert!(
                AgentTool::Claude
                    .new_session_command(&parsed, &session_id)
                    .is_none(),
                "`{command}` must not inject a session id into a wrapper"
            );
        }
    }

    /// A subcommand-based provider whose command already selects a subcommand
    /// cannot have `resume` injected before it (that would demote the original
    /// invocation to the resumed session's arguments), so it is refused - including
    /// a subcommand hidden behind a global option and its value. A flag, a value
    /// option, or a composition is not a subcommand and still resumes.
    #[test]
    fn an_existing_subcommand_refuses_an_injected_resume() {
        let native_id = NativeSessionId::try_new("thread-id").unwrap();

        for command in [
            r#"codex exec "task""#,
            r#"codex --profile work exec "task""#,
            "codex -m opus exec",
            "codex --model=opus exec",
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert!(
                AgentTool::Codex
                    .resume_command(&parsed, &native_id)
                    .is_none(),
                "`{command}` selects a subcommand and must not receive an injected resume"
            );
        }
        for command in [
            "codex --model opus",
            "codex --oss",
            "codex | tee log",
            "codex # use defaults",
            r#"codex "fix auth""#,
            "codex 'fix bug'",
            r#"codex --model opus "fix auth""#,
        ] {
            let parsed = CommandLine::try_new(command).unwrap();
            assert!(
                AgentTool::Codex
                    .resume_command(&parsed, &native_id)
                    .is_some(),
                "`{command}` selects no subcommand and must resume"
            );
        }
    }

    /// A quoted assignment value with whitespace is one shell token, so the
    /// provider arguments still land after the provider, not inside the quotes.
    #[test]
    fn provider_arguments_survive_a_quoted_assignment_value() {
        let command = CommandLine::try_new(r#"MODEL="two words" claude"#).unwrap();
        let session_id = AgentSessionId::try_new("assigned").unwrap();

        assert_eq!(
            AgentTool::Claude
                .new_session_command(&command, &session_id)
                .unwrap()
                .as_ref(),
            r#"MODEL="two words" claude --session-id assigned"#
        );
    }

    /// A POSIX backslash-escaped space keeps its assignment value one token, so the
    /// provider executable is found after it rather than mistaking the second word
    /// for the executable and refusing injection.
    #[cfg(not(windows))]
    #[test]
    fn provider_arguments_survive_an_escaped_assignment_value() {
        let command = CommandLine::try_new(r"MODEL=two\ words claude").unwrap();
        let session_id = AgentSessionId::try_new("assigned").unwrap();

        assert_eq!(
            AgentTool::Claude
                .new_session_command(&command, &session_id)
                .unwrap()
                .as_ref(),
            r"MODEL=two\ words claude --session-id assigned"
        );
    }
}
