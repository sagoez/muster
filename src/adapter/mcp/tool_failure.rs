use humantime::format_duration;
use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};

use crate::{
    adapter::{
        cli::{CliError, output},
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

/// Why a tool call did not succeed. The caller's mistake is answered as a
/// result flagged `isError`, which the agent reads and can act on; a fault of
/// muster's is answered as a protocol error.
#[derive(Debug)]
pub enum ToolFailure {
    /// The request was wrong, such as an unknown name or a blank key.
    Caller(CliError),
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
                let mut result =
                    CallToolResult::structured_error(output::failure(&error, &message));
                result.content = vec![ContentBlock::text(message)];
                Ok(result)
            },
            Self::Server(error) => Err(error),
        }
    }
}

impl From<CliError> for ToolFailure {
    fn from(error: CliError) -> Self {
        if error.kind().is_caller_fault() {
            return Self::Caller(error);
        }
        let message = agent_message(&error);
        let data = output::failure(&error, &message);
        Self::Server(ErrorData::internal_error(message, Some(data)))
    }
}

impl From<ConfigError> for ToolFailure {
    /// Classifies a store's refusal as a [`CliError`] does, so a wrong name
    /// or value is the caller's mistake and an unreadable database muster's.
    fn from(error: ConfigError) -> Self {
        CliError::from(error).into()
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

/// `error` worded for an agent: what to do names a tool to call, never a
/// command line, and a version skew asks for the MCP server to be restarted.
/// Every variant is matched, so a new one must be worded here before it
/// compiles. The variants passed through as they are say what went wrong and
/// name no command; `Usage` is only built by this server, never by the
/// command-line parser.
pub fn agent_message(error: &CliError) -> String {
    match error {
        CliError::NotRunning(name) => {
            format!(
                "{} is not running; {CALL_PROCESS_START}",
                name.shell_quoted()
            )
        },
        CliError::ServiceNewer { ours, theirs, .. } => format!(
            "the running muster service is version {theirs}, newer than this MCP server's {ours}; {RESTART_SERVER}"
        ),
        CliError::VersionMismatch { ours, theirs } => format!(
            "the muster service runs version {theirs}, not this MCP server's {ours}; {RESTART_SERVER}"
        ),
        CliError::UnexpectedAnswer(answer) => format!(
            "the muster service answered with `{answer}`, which this MCP server does not expect; {RESTART_SERVER}"
        ),
        CliError::Refused { kind, message } => refusal_message(*kind, message),
        CliError::Config(config) => config_message(config),
        CliError::NotAProject { dir } => format!(
            "no muster project in {} or above; {CALL_PROCESS_ADD}",
            dir.display()
        ),
        CliError::ProcessNameTaken { name, project } => format!(
            "a process named {} already exists in {}; {CHOOSE_ANOTHER_NAME}",
            name.shell_quoted(),
            project.display()
        ),
        CliError::NotPickedUp {
            name,
            project,
            waited,
        } => format!(
            "{} was added to {}, but the muster service did not pick it up within {}; {CALL_PROCESS_START}",
            name.shell_quoted(),
            project.display(),
            format_duration(*waited)
        ),
        CliError::NotATerminal => {
            "attaching needs a terminal; call process_output to read the process instead"
                .to_string()
        },
        CliError::AttachLost { name, source } => format!(
            "lost the connection to {}: {source}; call process_list to see whether it still runs",
            name.shell_quoted()
        ),
        CliError::CannotAsk { question, .. } => {
            format!("cannot ask {question} here; give it in the tool's arguments")
        },
        CliError::NothingRunning(project) => format!(
            "no process of {} is running; {CALL_PROCESS_START}",
            project.display()
        ),
        CliError::NothingToChoose(_)
        | CliError::Cancelled
        | CliError::Interrupted
        | CliError::InterruptedUnsent
        | CliError::UnknownProject(_)
        | CliError::InvalidNoteKey(_)
        | CliError::UnknownNote(_)
        | CliError::InvalidTodoTitle(_)
        | CliError::InvalidTodoId(_)
        | CliError::UnknownTodo(_)
        | CliError::InvalidKvKey(_)
        | CliError::UnknownValue(_)
        | CliError::InvalidName(_)
        | CliError::EmptyCommand
        | CliError::InvalidCommand
        | CliError::NothingToChange
        | CliError::ImportConflict { .. }
        | CliError::StackFile { .. }
        | CliError::UnsupportedStackVersion { .. }
        | CliError::FileExists { .. }
        | CliError::InvalidSetting { .. }
        | CliError::UnrepresentablePath(_)
        | CliError::InvalidProjectFolder(_)
        | CliError::UnknownProjectAmong { .. }
        | CliError::AmbiguousProject { .. }
        | CliError::NoStateDir
        | CliError::ServiceUnavailable { .. }
        | CliError::ServiceIo { .. }
        | CliError::Usage(_)
        | CliError::UnknownProcess { .. }
        | CliError::AmbiguousProcess { .. }
        | CliError::Log { .. }
        | CliError::Output(_)
        | CliError::Terminal(_) => error.to_string(),
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
        ConfigError::Write { .. }
        | ConfigError::NoStateDir
        | ConfigError::CoordinationId(_)
        | ConfigError::Database(_)
        | ConfigError::UnsupportedSchema { .. }
        | ConfigError::UnknownProcess { .. }
        | ConfigError::ImportConflict { .. }
        | ConfigError::InvalidSetting { .. }
        | ConfigError::InvalidDefaultSize { .. }
        | ConfigError::RootNotADirectory { .. } => error.to_string(),
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
        | ServiceErrorKind::InvalidConfig
        | ServiceErrorKind::NotRunning
        | ServiceErrorKind::Refused
        | ServiceErrorKind::Internal => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::domain::value::ProcessName;

    /// Failures whose command-line message names a muster command or flag.
    fn command_naming_failures() -> Vec<CliError> {
        let api = ProcessName::try_new("api").unwrap();
        vec![
            CliError::NotRunning(api.clone()),
            CliError::NotAProject {
                dir: PathBuf::from("/repo"),
            },
            CliError::ProcessNameTaken {
                name: api.clone(),
                project: PathBuf::from("/repo"),
            },
            CliError::Config(ConfigError::UnknownProject {
                root: PathBuf::from("/repo"),
            }),
            CliError::NotPickedUp {
                name: api,
                project: PathBuf::from("/repo"),
                waited: Duration::from_secs(5),
            },
            CliError::NotATerminal,
            CliError::ServiceNewer {
                ours: "0.9.1".to_string(),
                theirs: "9.0.0".to_string(),
                binary: None,
            },
            CliError::VersionMismatch {
                ours: "0.9.1".to_string(),
                theirs: "0.9.0".to_string(),
            },
            CliError::UnexpectedAnswer("ok"),
            CliError::Refused {
                kind: ServiceErrorKind::InvalidRequest,
                message: "unknown request".to_string(),
            },
        ]
    }

    /// A failure whose command-line message names a muster command or flag
    /// is reworded for an agent: a not-running process names the tool to
    /// call, and a version skew asks for a restart of the MCP server.
    #[test]
    fn agent_messages_name_tools_never_commands() {
        for error in command_naming_failures() {
            let cli = error.to_string();
            assert!(
                cli.contains("`muster") || cli.contains("--"),
                "the command line names a command: {cli}"
            );
            let message = agent_message(&error);
            assert!(!message.contains("`muster"), "{message}");
            assert!(!message.contains("--"), "{message}");
        }
        let api = ProcessName::try_new("api").unwrap();
        assert!(agent_message(&CliError::NotRunning(api)).contains("call process_start"));
        assert!(
            agent_message(&CliError::UnexpectedAnswer("ok")).contains(RESTART_SERVER),
            "a version skew is fixed by a restart"
        );
        let outside = CliError::Config(ConfigError::UnknownProject {
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
        let broken = ToolFailure::from(ConfigError::Database("disk I/O error".to_string()))
            .answer()
            .unwrap_err();
        assert_eq!(broken.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    }

    /// A wrong name is an error result carrying the valid names in its text
    /// and its structured content; a service that cannot be reached is a
    /// protocol error.
    #[test]
    fn a_wrong_name_is_an_error_result_and_an_unreachable_service_a_protocol_error() {
        let unknown = CliError::UnknownProcess {
            name: "echo".to_string(),
            project: PathBuf::from("/repo"),
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

        let unreachable = ToolFailure::from(CliError::NoStateDir)
            .answer()
            .unwrap_err();
        assert_eq!(unreachable.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            unreachable.data.expect("the failure rides along")["error"]["kind"],
            "service-unavailable"
        );
    }

    /// A store the service cannot read is muster's fault, answered as a
    /// protocol error rather than a mistake for the agent to fix.
    #[test]
    fn an_unreadable_store_is_a_protocol_error() {
        let unreadable = CliError::Refused {
            kind: ServiceErrorKind::Internal,
            message: "could not read this project's stack".to_string(),
        };

        let error = ToolFailure::from(unreadable).answer().unwrap_err();

        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    }
}
