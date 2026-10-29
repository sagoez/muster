use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use serde_json::Value;

use super::tool_error::ToolError;
use crate::{
    adapter::{
        client::{ClientError, FailureView},
        service::ServiceErrorKind,
    },
    domain::config::ConfigError,
};

/// What an agent does about a muster service that runs another version, since
/// no tool can fix it.
const RESTART_SERVER: &str = "restart the muster MCP server";
/// What an agent does about a service that is shutting down.
const CALL_AGAIN: &str = "the muster service is shutting down; call the tool again";
/// What an agent does about a process that is not running.
const CALL_PROCESS_START: &str = "call process_start to start it";
/// What an agent does in a folder that is not a project.
const CALL_PROCESS_ADD: &str = "call process_add to register this folder as a project";
/// What an agent does about a name the project already has.
const CHOOSE_ANOTHER_NAME: &str = "choose another name";
/// What an agent does about a note key that names no note.
const CALL_NOTE_READ: &str = "note_read without key lists the notes";
/// What an agent does about a todo id that names no todo.
const CALL_TODO_LIST: &str = "todo_list lists the todos";
/// What an agent does about a key that names no value.
const CALL_KV_READ: &str = "kv_read without key lists the entries";
/// What an agent does about a project name that names no project.
const PASS_PROJECT: &str = "pass a registered project's name or a folder as project";
/// What an agent does about a cursor past the end of the output.
const PASS_CURSOR: &str = "pass a cursor from an earlier result";

/// Why a tool call did not succeed. The caller's mistake is answered as a
/// result flagged `isError`, which the agent reads and can act on; a fault of
/// muster's is answered as a protocol error.
#[derive(Debug)]
pub enum ToolFailure {
    /// The request was wrong, such as an unknown name or a blank key.
    Caller(ToolError),
    /// muster could not answer it.
    Server(ErrorData),
}

impl ToolFailure {
    /// The tool's answer: an error result for the caller's mistake, or the
    /// protocol error for muster's fault.
    ///
    /// # Errors
    /// Returns the [`ErrorData`] of a fault of muster's.
    pub fn answer(self) -> Result<CallToolResult, ErrorData> {
        match self {
            Self::Caller(error) => {
                let message = agent_message(&error);
                let mut result = CallToolResult::structured_error(failure(&error, &message));
                result.content = vec![ContentBlock::text(message)];
                Ok(result)
            },
            Self::Server(error) => Err(error),
        }
    }
}

impl From<ToolError> for ToolFailure {
    fn from(error: ToolError) -> Self {
        if error.kind().is_caller_fault() {
            return Self::Caller(error);
        }
        let message = agent_message(&error);
        let data = failure(&error, &message);
        Self::Server(ErrorData::internal_error(message, Some(data)))
    }
}

impl From<ClientError> for ToolFailure {
    fn from(error: ClientError) -> Self {
        ToolError::from(error).into()
    }
}

impl From<ConfigError> for ToolFailure {
    /// Classifies a store's refusal by its cause, so a wrong name or value is
    /// the caller's mistake and an unreadable database muster's.
    fn from(error: ConfigError) -> Self {
        ToolError::from(error).into()
    }
}

impl From<ErrorData> for ToolFailure {
    fn from(error: ErrorData) -> Self {
        Self::Server(error)
    }
}

impl IntoCallToolResult for ToolFailure {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        self.answer().map(CallToolResponse::from)
    }
}

/// `error` as the JSON object a failure is reported as, with `message`.
fn failure(error: &ToolError, message: &str) -> Value {
    FailureView::builder()
        .kind(error.kind())
        .message(message.to_string())
        .candidates(error.candidates())
        .build()
        .to_json()
}

/// `error` worded for an agent: what to do names a tool to call, never a
/// command line, and a version skew asks for the MCP server to be restarted.
pub fn agent_message(error: &ToolError) -> String {
    match error {
        ToolError::Client(client) => client_message(client),
        ToolError::Config(config) => config_message(config),
        ToolError::UnknownNote { .. } => format!("{error}; {CALL_NOTE_READ}"),
        ToolError::UnknownTodo { .. } => format!("{error}; {CALL_TODO_LIST}"),
        ToolError::UnknownValue { .. } => format!("{error}; {CALL_KV_READ}"),
        ToolError::UnknownProject { .. } => format!("{error}; {PASS_PROJECT}"),
        ToolError::NoProjectHere { .. }
        | ToolError::Unregistrable { .. }
        | ToolError::OutsideBase { .. }
        | ToolError::ProjectRequired { .. }
        | ToolError::InvalidName { .. }
        | ToolError::EmptyCommand
        | ToolError::InvalidField { .. }
        | ToolError::InvalidPattern { .. }
        | ToolError::NothingToChange(_)
        | ToolError::InvalidNoteKey(_)
        | ToolError::InvalidTodoTitle(_)
        | ToolError::InvalidTodoId(_)
        | ToolError::HeldByOther { .. }
        | ToolError::InvalidKvKey(_)
        | ToolError::StackEditsReadOnly(_)
        | ToolError::SettingNotForAgents(_)
        | ToolError::WorkingDirOutsideProject { .. } => error.to_string(),
    }
}

/// A failure reaching the service worded for an agent. Every variant is
/// matched, so a new one must be worded here before it compiles.
fn client_message(error: &ClientError) -> String {
    match error {
        ClientError::NotRunning(_) => format!("{error}; {CALL_PROCESS_START}"),
        ClientError::ServiceNewer { .. }
        | ClientError::VersionMismatch { .. }
        | ClientError::UnexpectedAnswer(_) => format!("{error}; {RESTART_SERVER}"),
        ClientError::Refused { kind, message } => refusal_message(*kind, message),
        ClientError::Config(config) => config_message(config),
        ClientError::CursorAhead { .. } => format!("invalid since: {error}; {PASS_CURSOR}"),
        ClientError::NoStateDir(_)
        | ClientError::SocketPathTooLong { .. }
        | ClientError::ServiceUnavailable { .. }
        | ClientError::ServiceIo { .. }
        | ClientError::UnknownProcess { .. }
        | ClientError::AmbiguousProcess { .. }
        | ClientError::Log { .. }
        | ClientError::InterruptedUnsent
        | ClientError::Interrupted => error.to_string(),
    }
}

/// A store's refusal `error` worded for an agent: the two whose message names
/// a command name a tool instead.
fn config_message(error: &ConfigError) -> String {
    match error {
        ConfigError::UnknownProject { root } => format!(
            "no muster project at {}; {CALL_PROCESS_ADD}",
            root.display()
        ),
        ConfigError::DuplicateProcessName { name } => format!(
            "a process named {} already exists in this project; {CHOOSE_ANOTHER_NAME}",
            name.shell_quoted()
        ),
        ConfigError::CoordinationId(_)
        | ConfigError::Storage(_)
        | ConfigError::CorruptRow { .. }
        | ConfigError::TooLarge { .. }
        | ConfigError::UnsupportedSchema { .. }
        | ConfigError::UnknownProcess { .. }
        | ConfigError::ImportConflict { .. }
        | ConfigError::InvalidSetting { .. }
        | ConfigError::InvalidDefaultSize { .. }
        | ConfigError::RootNotADirectory { .. }
        | ConfigError::RootNotUtf8 { .. }
        | ConfigError::HoldsProject { .. } => error.to_string(),
    }
}

/// The service's refusal `message` of `kind`, with what an agent does about
/// it.
fn refusal_message(kind: ServiceErrorKind, message: &str) -> String {
    match kind {
        ServiceErrorKind::InvalidRequest => {
            format!("{message}; the muster service may run another version, {RESTART_SERVER}")
        },
        ServiceErrorKind::Busy => format!("{message}; {CALL_AGAIN}"),
        ServiceErrorKind::UnknownProcess
        | ServiceErrorKind::UnknownProject
        | ServiceErrorKind::NotRunning
        | ServiceErrorKind::InvalidInput
        | ServiceErrorKind::Internal => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rmcp::model::ErrorCode;

    use super::*;
    use crate::{
        adapter::no_state_dir::NoStateDir,
        domain::value::{ProcessName, ProjectName},
    };

    /// Failures whose command-line wording names a muster command or flag.
    fn command_naming_failures() -> Vec<ToolError> {
        let api = ProcessName::try_new("api").unwrap();
        vec![
            ClientError::NotRunning(api).into(),
            ConfigError::UnknownProject {
                root: PathBuf::from("/repo"),
            }
            .into(),
            ClientError::ServiceNewer {
                ours: "0.9.1".to_string(),
                theirs: "9.0.0".to_string(),
                binary: None,
            }
            .into(),
            ClientError::VersionMismatch {
                ours: "0.9.1".to_string(),
                theirs: "0.9.0".to_string(),
            }
            .into(),
            ClientError::UnexpectedAnswer("ok").into(),
            ClientError::Refused {
                kind: ServiceErrorKind::InvalidRequest,
                message: "unknown request".to_string(),
            }
            .into(),
        ]
    }

    /// A failure is worded for an agent: a not-running process names the
    /// tool to call, a version skew asks for a restart of the MCP server,
    /// and no message names a command line.
    #[test]
    fn agent_messages_name_tools_never_commands() {
        for error in command_naming_failures() {
            let message = agent_message(&error);
            assert!(!message.contains("`muster"), "{message}");
            assert!(!message.contains("--"), "{message}");
        }
        let api = ProcessName::try_new("api").unwrap();
        assert!(agent_message(&ClientError::NotRunning(api).into()).contains("call process_start"));
        assert!(
            agent_message(&ClientError::UnexpectedAnswer("ok").into()).contains(RESTART_SERVER),
            "a version skew is fixed by a restart"
        );
        let outside = ToolError::from(ConfigError::UnknownProject {
            root: PathBuf::from("/repo"),
        });
        assert!(
            agent_message(&outside).contains("call process_add"),
            "a folder that is not a project is registered by process_add"
        );
    }

    /// A store's refusal of a wrong name is the caller's mistake listing the
    /// valid names; a database it cannot read is muster's fault.
    #[test]
    fn a_store_refusal_is_classified_by_its_cause() {
        let unknown = ConfigError::UnknownProcess {
            name: ProcessName::try_new("wbe").unwrap(),
            valid: vec![ProcessName::try_new("web").unwrap()],
        };

        let result = ToolFailure::from(unknown).answer().unwrap();

        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.expect("structured")["error"]["candidates"],
            serde_json::json!(["web"])
        );
        let broken = ToolFailure::from(ConfigError::Storage("disk I/O error".into()))
            .answer()
            .unwrap_err();
        assert_eq!(broken.code, ErrorCode::INTERNAL_ERROR);
    }

    /// A wrong name is an error result carrying the valid names in its text
    /// and its structured content; a service that cannot be reached is a
    /// protocol error.
    #[test]
    fn a_wrong_name_is_an_error_result_and_an_unreachable_service_a_protocol_error() {
        let unknown = ClientError::UnknownProcess {
            name: "echo".to_string(),
            project: ProjectName::try_new("repo").unwrap(),
            valid: vec![
                ProcessName::try_new("echo box").unwrap(),
                ProcessName::try_new("api").unwrap(),
            ],
        };

        let result = ToolFailure::from(unknown).answer().unwrap();

        assert_eq!(result.is_error, Some(true));
        let ContentBlock::Text(text) = &result.content[0] else {
            panic!("a text block");
        };
        assert!(text.text.contains("'echo box'"), "{}", text.text);
        let error = &result.structured_content.expect("structured")["error"];
        assert_eq!(error["kind"], "unknown-process");
        assert_eq!(error["candidates"], serde_json::json!(["echo box", "api"]));

        let unreachable = ToolFailure::from(ClientError::NoStateDir(NoStateDir))
            .answer()
            .unwrap_err();
        assert_eq!(unreachable.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            unreachable.data.expect("the failure rides along")["error"]["kind"],
            "service-unavailable"
        );
    }

    /// A store the service cannot read is muster's fault, answered as a
    /// protocol error rather than a mistake for the agent to fix.
    #[test]
    fn an_unreadable_store_is_a_protocol_error() {
        let unreadable = ClientError::Refused {
            kind: ServiceErrorKind::Internal,
            message: "could not read this project's stack".to_string(),
        };

        let error = ToolFailure::from(unreadable).answer().unwrap_err();

        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
    }
}
