use std::path::PathBuf;

use getset::Getters;
use nutype::nutype;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{AgentIdentitySource, AgentProtocol, AgentTool},
    value::{CommandLine, ProcessName},
};

/// Stable Muster-owned identity for an agent session.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct AgentSessionId(String);

impl AgentSessionId {
    /// Creates a globally unique identity for a new session.
    ///
    /// # Errors
    /// Returns the generated newtype's validation error if its invariant ever
    /// diverges from UUID's non-empty textual representation.
    pub fn generate() -> Result<Self, AgentSessionIdError> {
        Self::try_new(uuid::Uuid::new_v4().to_string())
    }
}

/// Provider-owned identity used to resume an existing conversation.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct NativeSessionId(String);

/// Operating-system process identity of the provider instance owned by Muster.
#[nutype(
    validate(greater = 0),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Hash,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct AgentProcessId(u32);

/// Non-reusable creation marker paired with an operating-system process ID.
#[nutype(
    validate(greater = 0),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Hash,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct AgentProcessStartToken(u64);

/// Whether a persisted session should be restored with its workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSessionState {
    /// A fresh launch was reserved but has not attached to a PTY successfully.
    Pending,
    /// The pane was open when Muster last owned the workspace.
    Open,
    /// The user closed the pane, leaving it available in history.
    Closed,
}

/// POSIX shell lexical context at a resume-template byte offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellContext {
    /// The placeholder is parsed as ordinary shell syntax.
    Unquoted,
    /// The placeholder appears inside single quotes.
    Single,
    /// The placeholder appears inside double quotes.
    Double,
    /// The placeholder appears inside legacy backtick command substitution.
    Backtick,
    /// A terminal backslash has no following character to escape.
    TrailingEscape,
}

/// Placeholder accepted by custom provider resume templates.
const SESSION_ID_PLACEHOLDER: &str = "{session_id}";

/// Durable metadata for one provider-backed agent conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct AgentSession {
    /// Stable identity owned by Muster rather than the provider.
    id: AgentSessionId,
    /// Generated or user-supplied display name.
    name: ProcessName,
    /// Provider preset controlling launch, activity, and resume behavior.
    tool: AgentTool,
    /// Exact workspace config location that owns the session.
    project: PathBuf,
    /// Original launch command, including user customizations.
    launch_command: CommandLine,
    /// Optional working directory relative to the owning workspace.
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Optional custom resume template. `{session_id}` is replaced when present.
    #[builder(default)]
    resume_command: Option<CommandLine>,
    /// Provider identity captured by a lifecycle integration.
    #[builder(default)]
    native_id: Option<NativeSessionId>,
    /// Process identity allowed to update the provider conversation.
    #[builder(default)]
    owner_process_id: Option<AgentProcessId>,
    /// Creation marker paired with the owner PID to reject PID reuse.
    #[builder(default)]
    owner_process_start_token: Option<AgentProcessStartToken>,
    /// Shell wrapper identity allowed to hand off to its direct provider child.
    #[builder(default)]
    wrapper_process_id: Option<AgentProcessId>,
    /// Open sessions restore automatically; closed sessions remain in history.
    state: AgentSessionState,
}

impl AgentSession {
    /// Returns a copy with the provider identity captured by a hook or plugin.
    pub fn with_native_id(mut self, native_id: NativeSessionId) -> Self {
        self.native_id = Some(native_id);
        self
    }

    /// Returns a copy bound to the current managed provider process.
    pub fn with_owner_process_id(mut self, process_id: AgentProcessId) -> Self {
        self.owner_process_id = Some(process_id);
        self
    }

    /// Returns a copy bound to a launch owner and its optional shell wrapper.
    pub fn with_launch_processes(
        mut self,
        owner_process_id: AgentProcessId,
        owner_process_start_token: Option<AgentProcessStartToken>,
        wrapper_process_id: Option<AgentProcessId>,
    ) -> Self {
        self.owner_process_id = Some(owner_process_id);
        self.owner_process_start_token = owner_process_start_token;
        self.wrapper_process_id = wrapper_process_id;
        self
    }

    /// Returns a copy with the requested persisted lifecycle state.
    pub fn with_state(mut self, state: AgentSessionState) -> Self {
        self.state = state;
        self
    }

    /// Builds the provider-specific command that resumes this conversation.
    pub fn resume(&self) -> Option<CommandLine> {
        let native_id = self.native_id.as_ref()?;
        if let Some(template) = &self.resume_command {
            return Self::expand_resume_template(template, native_id);
        }
        self.tool.resume_command(&self.launch_command, native_id)
    }

    /// Builds the safest command available after a process or Muster restart.
    /// Pending launches and open caller-assigned identities retry their original
    /// new-session command until a lifecycle hook confirms them.
    pub fn restore_command(&self) -> Option<CommandLine> {
        if self.native_id.is_some() {
            return self.resume();
        }
        (self.state == AgentSessionState::Pending
            || self.state == AgentSessionState::Open
                && self.tool.identity_source() == AgentIdentitySource::Assigned)
            .then(|| {
                self.tool
                    .new_session_command(&self.launch_command, &self.id)
            })
            .flatten()
    }

    /// Whether a custom resume template can accept a provider identity without
    /// embedding it in a quoted or compound shell word. The complete template
    /// must also end outside quotes or escape syntax.
    pub fn resume_template_is_valid(template: &CommandLine) -> bool {
        let template = template.as_ref();
        Self::shell_context(template) == ShellContext::Unquoted
            && !Self::contains_here_document(template)
            && (!template.contains(SESSION_ID_PLACEHOLDER)
                && Self::command_text_accepts_provider_arguments(template)
                || template.contains(SESSION_ID_PLACEHOLDER)
                    && template
                        .match_indices(SESSION_ID_PLACEHOLDER)
                        .all(|(index, _)| {
                            Self::placeholder_is_unquoted_shell_word(
                                template,
                                index,
                                SESSION_ID_PLACEHOLDER,
                            )
                        }))
    }

    /// Whether built-in provider arguments can be safely appended to `command`.
    /// Shell compositions require an explicit resume template so arguments are
    /// never attached to a different command in a pipeline or sequence.
    pub fn launch_command_accepts_provider_arguments(command: &CommandLine) -> bool {
        Self::command_text_accepts_provider_arguments(command.as_ref())
    }

    /// Whether provider arguments can be appended to shell command text.
    fn command_text_accepts_provider_arguments(command: &str) -> bool {
        let mut context = ShellContext::Unquoted;
        let mut chars = command.chars();
        while let Some(character) = chars.next() {
            if context == ShellContext::Unquoted
                && matches!(
                    character,
                    '#' | '|' | ';' | '&' | '(' | ')' | '`' | '\n' | '\r'
                )
            {
                return false;
            }
            context = match (context, character) {
                (ShellContext::Unquoted, '\\')
                | (ShellContext::Double, '\\')
                | (ShellContext::Backtick, '\\') => {
                    if chars.next().is_some() {
                        context
                    } else {
                        ShellContext::TrailingEscape
                    }
                },
                (ShellContext::Unquoted, '\'') => ShellContext::Single,
                (ShellContext::Single, '\'') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '"') => ShellContext::Double,
                (ShellContext::Double, '"') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '`') => ShellContext::Backtick,
                (ShellContext::Backtick, '`') => ShellContext::Unquoted,
                _ => context,
            };
        }
        context == ShellContext::Unquoted
    }

    /// Returns whether unquoted text introduces a here-document body, where
    /// shell quoting cannot safely protect a substituted provider identity.
    fn contains_here_document(command: &str) -> bool {
        let mut context = ShellContext::Unquoted;
        let mut chars = command.chars().peekable();
        while let Some(character) = chars.next() {
            if context == ShellContext::Unquoted && character == '<' && chars.peek() == Some(&'<') {
                return true;
            }
            context = match (context, character) {
                (ShellContext::Unquoted, '\\')
                | (ShellContext::Double, '\\')
                | (ShellContext::Backtick, '\\') => {
                    if chars.next().is_some() {
                        context
                    } else {
                        ShellContext::TrailingEscape
                    }
                },
                (ShellContext::Unquoted, '\'') => ShellContext::Single,
                (ShellContext::Single, '\'') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '"') => ShellContext::Double,
                (ShellContext::Double, '"') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '`') => ShellContext::Backtick,
                (ShellContext::Backtick, '`') => ShellContext::Unquoted,
                _ => context,
            };
        }
        false
    }

    /// Expands a custom resume command, accepting the placeholder only as an
    /// unquoted standalone shell word, or appending a safely quoted ID.
    fn expand_resume_template(
        template: &CommandLine,
        native_id: &NativeSessionId,
    ) -> Option<CommandLine> {
        let quoted = Self::quote_for_command_shell(native_id.as_ref())?;
        Self::resume_template_is_valid(template).then_some(())?;
        let template = template.as_ref();
        let command = if template.contains(SESSION_ID_PLACEHOLDER) {
            template.replace(SESSION_ID_PLACEHOLDER, &quoted)
        } else {
            format!("{template} {quoted}")
        };
        CommandLine::try_new(command).ok()
    }

    /// Quotes one opaque argument for the command shell that launches agents.
    pub(crate) fn quote_for_command_shell(value: &str) -> Option<String> {
        #[cfg(windows)]
        {
            // cmd.exe expands percent variables even inside double quotes. Its
            // ordinary metacharacters are inert there, so double percent signs
            // preserve an opaque provider identity as one command argument.
            Some(format!(
                "\"{}\"",
                value.replace('%', "%%").replace('"', "^\"")
            ))
        }
        #[cfg(not(windows))]
        {
            shlex::try_quote(value).ok().map(Into::into)
        }
    }

    /// Whether the placeholder is an unquoted shell word delimited by command
    /// boundaries or unescaped whitespace.
    fn placeholder_is_unquoted_shell_word(template: &str, index: usize, placeholder: &str) -> bool {
        let before = &template[..index];
        let after = &template[index + placeholder.len()..];
        Self::shell_context(before) == ShellContext::Unquoted
            && Self::prefix_ends_at_shell_boundary(before)
            && after.chars().next().is_none_or(char::is_whitespace)
    }

    /// Whether `prefix` ends at a command boundary or whitespace that is not
    /// escaped by an odd-length backslash run.
    fn prefix_ends_at_shell_boundary(prefix: &str) -> bool {
        let Some(last) = prefix.chars().next_back() else {
            return true;
        };
        if !last.is_whitespace() {
            return false;
        }
        prefix
            .chars()
            .rev()
            .skip(1)
            .take_while(|character| *character == '\\')
            .count()
            % 2
            == 0
    }

    /// Tracks POSIX shell lexical state through `prefix`, treating complete
    /// escapes as literals and retaining an incomplete terminal escape.
    fn shell_context(prefix: &str) -> ShellContext {
        let mut context = ShellContext::Unquoted;
        let mut chars = prefix.chars();
        while let Some(character) = chars.next() {
            context = match (context, character) {
                (ShellContext::Unquoted, '\\')
                | (ShellContext::Double, '\\')
                | (ShellContext::Backtick, '\\') => {
                    if chars.next().is_some() {
                        context
                    } else {
                        ShellContext::TrailingEscape
                    }
                },
                (ShellContext::Unquoted, '\'') => ShellContext::Single,
                (ShellContext::Single, '\'') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '"') => ShellContext::Double,
                (ShellContext::Double, '"') => ShellContext::Unquoted,
                (ShellContext::Unquoted, '`') => ShellContext::Backtick,
                (ShellContext::Backtick, '`') => ShellContext::Unquoted,
                _ => context,
            };
        }
        context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A custom resume template substitutes its provider identity safely.
    #[test]
    fn expands_custom_resume_templates() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(
                CommandLine::try_new("agent --resume {session_id}").unwrap(),
            ))
            .native_id(Some(NativeSessionId::try_new("thread one").unwrap()))
            .state(AgentSessionState::Closed)
            .build();

        assert_eq!(
            session.resume().unwrap().as_ref(),
            "agent --resume 'thread one'"
        );
    }

    /// A placeholder-free template appends the provider identity as one safely
    /// quoted shell word when the template ends in ordinary shell context.
    #[test]
    fn appends_ids_to_complete_placeholder_free_templates() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(CommandLine::try_new("agent --resume").unwrap()))
            .native_id(Some(NativeSessionId::try_new("thread one").unwrap()))
            .state(AgentSessionState::Closed)
            .build();

        assert_eq!(
            session.resume().unwrap().as_ref(),
            "agent --resume 'thread one'"
        );
    }

    /// Placeholder-free templates with an unmatched quote or dangling escape
    /// are rejected before provider-controlled identity text can be appended.
    #[test]
    fn rejects_incomplete_placeholder_free_templates() {
        for template in ["agent --resume \"", "agent --resume \\"] {
            let template = CommandLine::try_new(template).unwrap();
            assert!(!AgentSession::resume_template_is_valid(&template));
        }
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(CommandLine::try_new("agent --resume \"").unwrap()))
            .native_id(Some(
                NativeSessionId::try_new("\"; some-command; #").unwrap(),
            ))
            .state(AgentSessionState::Closed)
            .build();

        assert!(session.resume().is_none());
    }

    /// Built-in provider flags are never appended to a pipeline or sequence.
    #[test]
    fn rejects_shell_compositions_for_provider_arguments() {
        let pipeline = CommandLine::try_new("codex | tee agent.log").unwrap();
        let sequence = CommandLine::try_new("codex; echo done").unwrap();
        let newline = CommandLine::try_new("codex\ntee agent.log").unwrap();
        let simple = CommandLine::try_new("FOO=bar codex").unwrap();

        assert!(!AgentSession::launch_command_accepts_provider_arguments(
            &pipeline
        ));
        assert!(!AgentSession::launch_command_accepts_provider_arguments(
            &sequence
        ));
        assert!(!AgentSession::launch_command_accepts_provider_arguments(
            &newline
        ));
        assert!(AgentSession::launch_command_accepts_provider_arguments(
            &simple
        ));
    }

    /// Placeholder-free templates cannot append an ID after a composition or
    /// comment, while an explicit placeholder remains safe before a pipeline.
    #[test]
    fn rejects_placeholder_free_composed_resume_templates() {
        for template in [
            "agent --resume | tee agent.log",
            "agent --resume # local",
            "agent --resume\ntee agent.log",
        ] {
            let template = CommandLine::try_new(template).unwrap();
            assert!(!AgentSession::resume_template_is_valid(&template));
        }
        let explicit = CommandLine::try_new("agent --resume {session_id} | tee agent.log").unwrap();
        assert!(AgentSession::resume_template_is_valid(&explicit));
    }

    /// Here-document bodies do not apply shell quoting to substituted values.
    #[test]
    fn rejects_resume_placeholders_inside_here_documents() {
        let template = CommandLine::try_new("cat <<EOF\n{session_id}\nEOF").unwrap();

        assert!(!AgentSession::resume_template_is_valid(&template));
    }

    /// A placeholder nested inside user-provided quotes is rejected because a
    /// pre-quoted shell word cannot be inserted safely into that context.
    #[test]
    fn rejects_a_quoted_resume_placeholder() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(
                CommandLine::try_new("agent --resume \"{session_id}\"").unwrap(),
            ))
            .native_id(Some(NativeSessionId::try_new("thread one").unwrap()))
            .state(AgentSessionState::Closed)
            .build();

        assert!(session.resume().is_none());
    }

    /// Whitespace around a placeholder does not make it safe when the whole
    /// placeholder still appears inside a double-quoted shell word.
    #[test]
    fn rejects_a_spaced_placeholder_inside_shell_quotes() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(
                CommandLine::try_new("agent --resume \"prefix {session_id} suffix\"").unwrap(),
            ))
            .native_id(Some(
                NativeSessionId::try_new("$(touch /tmp/muster-owned)").unwrap(),
            ))
            .state(AgentSessionState::Closed)
            .build();

        assert!(session.resume().is_none());
    }

    /// Shell metacharacters in provider-owned IDs remain inside one quoted word.
    #[test]
    fn quotes_shell_metacharacters_in_resume_ids() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Custom)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("agent").unwrap())
            .resume_command(Some(
                CommandLine::try_new("agent --resume {session_id}").unwrap(),
            ))
            .native_id(Some(
                NativeSessionId::try_new("$(touch /tmp/muster-owned)").unwrap(),
            ))
            .state(AgentSessionState::Closed)
            .build();

        assert_eq!(
            session.resume().unwrap().as_ref(),
            "agent --resume '$(touch /tmp/muster-owned)'"
        );
    }

    /// Windows must use cmd.exe quoting rather than POSIX single quotes.
    #[cfg(windows)]
    #[test]
    fn quotes_resume_ids_for_the_windows_command_shell() {
        assert_eq!(
            AgentSession::quote_for_command_shell("thread & command"),
            Some("\"thread & command\"".to_string())
        );
    }

    /// A caller-assigned provider retries its stable launch identity until the
    /// provider confirms that conversation through a lifecycle hook.
    #[test]
    fn restores_an_unconfirmed_assigned_identity_with_a_new_session_command() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("assigned-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Claude)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .state(AgentSessionState::Open)
            .build();

        assert!(session.resume().is_none());
        assert_eq!(
            session.restore_command().unwrap().as_ref(),
            "claude --session-id assigned-session"
        );
    }

    /// A closed unconfirmed conversation never becomes a fresh launch from history.
    #[test]
    fn does_not_reopen_a_closed_unconfirmed_assigned_identity() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("closed-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Claude)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .state(AgentSessionState::Closed)
            .build();

        assert!(session.restore_command().is_none());
    }

    /// A reported-ID session retries safely when its initial PTY launch never attached.
    #[test]
    fn restores_a_pending_reported_identity_with_a_new_session_command() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("pending-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Codex)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Pending)
            .build();

        assert_eq!(session.restore_command().unwrap().as_ref(), "codex");
    }

    /// Once a caller-assigned provider confirms its native conversation, all
    /// restoration paths use the provider's resume command.
    #[test]
    fn restores_a_confirmed_assigned_identity_with_resume() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("assigned-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Claude)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .native_id(Some(NativeSessionId::try_new("confirmed-session").unwrap()))
            .state(AgentSessionState::Open)
            .build();

        assert_eq!(
            session.restore_command().unwrap().as_ref(),
            "claude --resume confirmed-session"
        );
    }

    /// A confirmed identity with invalid durable resume behavior never falls
    /// back to a fresh conversation under the same Muster session.
    #[test]
    fn does_not_replace_a_confirmed_conversation_when_resume_is_invalid() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("assigned-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Claude)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .resume_command(Some(CommandLine::try_new("claude --resume \"").unwrap()))
            .native_id(Some(NativeSessionId::try_new("confirmed-session").unwrap()))
            .state(AgentSessionState::Open)
            .build();

        assert!(session.restore_command().is_none());
    }
}
