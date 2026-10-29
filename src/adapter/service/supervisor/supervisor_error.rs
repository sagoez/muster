use thiserror::Error;

use crate::domain::{config::ConfigError, value::ProcessName};

/// Shown when durable agent-session state cannot be loaded or written.
pub(crate) const AGENT_SESSION_STORE_ERROR: &str = "could not update agent session history";
/// Shown when a closed session has no provider identity to resume.
pub(crate) const AGENT_SESSION_NOT_RESUMABLE: &str = "the provider never reported this session's ID; check `muster doctor`, approve hooks in the provider (codex: /hooks), then open a new session";
/// Shown when a custom agent session has no launch command.
pub(crate) const AGENT_COMMAND_REQUIRED: &str = "a custom agent needs a command";
/// Shown when a custom resume placeholder cannot be expanded safely.
pub(crate) const AGENT_RESUME_TEMPLATE_INVALID: &str =
    "resume placeholder must be an unquoted standalone shell word";
/// Shown, before the name, when a process's name is already used by another
/// process.
const NAME_TAKEN: &str = "another process already uses the name";
/// Why a taken name is refused.
const NAMES_UNIQUE: &str = "process names are unique within a project";
/// Shown when there is no closed resumable session in this workspace.
pub(crate) const NO_RECENT_AGENT_SESSION: &str = "no closed agent session to reopen";

/// Why a supervisor request was refused. Its message is what the user sees.
#[derive(Debug, Error)]
pub enum SupervisorError {
    /// A custom agent was given no command to launch.
    #[error("{message}", message = AGENT_COMMAND_REQUIRED)]
    CommandRequired,
    /// A custom resume template cannot be expanded safely.
    #[error("{message}", message = AGENT_RESUME_TEMPLATE_INVALID)]
    ResumeTemplateInvalid,
    /// No session identity or display name could be generated.
    #[error("{message}", message = AGENT_SESSION_STORE_ERROR)]
    SessionIdentityUnavailable,
    /// Durable agent-session history could not be read or written.
    #[error("{prefix}: {0}", prefix = AGENT_SESSION_STORE_ERROR)]
    SessionStore(ConfigError),
    /// The session has no provider identity to resume.
    #[error("{message}", message = AGENT_SESSION_NOT_RESUMABLE)]
    NotResumable,
    /// No closed session of this project can be reopened.
    #[error("{message}", message = NO_RECENT_AGENT_SESSION)]
    NoRecentSession,
    /// Another process of the project already uses the name.
    #[error("{prefix} '{0}'; {rule}", prefix = NAME_TAKEN, rule = NAMES_UNIQUE)]
    NameTaken(ProcessName),
}
