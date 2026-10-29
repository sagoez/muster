use std::{io, path::PathBuf, time::Duration};

use thiserror::Error;

use crate::{
    adapter::{
        client::{ClientError, FailureKind, ResolveError},
        exchange::ExchangeError,
        holder::{HolderError, PatternError},
        service::ServiceErrorKind,
    },
    domain::{
        config::ConfigError,
        coordination::Author,
        value::{ProcessName, ProjectName},
    },
};

/// What a person should try after a refusal of each kind, appended to its
/// message.
const BUSY_HINT: &str = "the muster service is shutting down; run the command again";
const INVALID_REQUEST_HINT: &str = "the running service may be another muster version; `muster service stop` and run the command again";
/// What a person does about a service that runs another version.
const STOP_SERVICE_HINT: &str = "`muster service stop` and run the command again";
/// What a person does about a service newer than this binary.
const NEWER_SERVICE_HINT: &str =
    "run the newer muster, or `muster service stop` to let this one start its own";
/// Where a person sees how interrupted processes end.
const INTERRUPTED_HINT: &str = "`muster ps` shows how they end";
/// Where a person finds a cursor that names output.
const CURSOR_HINT: &str = "pass a cursor an earlier `muster wait` or `muster logs --json` printed";
/// What `muster run` suggests after a taken name.
pub const RUN_NAME_HINT: &str = "; pass --name to choose another";

/// A failure while executing a CLI command.
#[derive(Debug, Error)]
pub enum CliError {
    /// `--project` named no registered project.
    #[error("no registered project is named '{0}'; `muster projects` lists them")]
    UnknownProject(String),
    /// The command acts on a project and none contains the folder it ran in.
    #[error(
        "no muster project in {} or above; `muster add <name> -- <command>` starts one",
        dir.display()
    )]
    NotAProject {
        /// The folder the project was looked for from.
        dir: PathBuf,
    },
    /// A note key was blank or whitespace-only.
    #[error("'{0}' is not a valid note key")]
    InvalidNoteKey(String),
    /// No note is stored under the requested key.
    #[error("no note '{0}'")]
    UnknownNote(String),
    /// A todo title was blank or whitespace-only.
    #[error("'{0}' is not a valid todo title")]
    InvalidTodoTitle(String),
    /// A todo id was blank or whitespace-only.
    #[error("'{0}' is not a valid todo id")]
    InvalidTodoId(String),
    /// No todo exists with the requested id.
    #[error("no todo '{0}'")]
    UnknownTodo(String),
    /// Another agent holds what the command would change under a live lease.
    #[error(
        "{what} is held by {owner} for {}; pass --force to change it anyway",
        humantime::format_duration(*expires_in)
    )]
    HeldByOther {
        /// What is held, as `todo '<id>'`.
        what: String,
        /// Who holds it.
        owner: Author,
        /// How long the lease has left.
        expires_in: Duration,
    },
    /// A lock name was not 1 to 64 characters of `[a-z0-9._-]`.
    #[error("'{0}' is not a valid lock name; use 1 to 64 of a-z, 0-9, '.', '_' and '-'")]
    InvalidLockName(String),
    /// A key-value key was blank or whitespace-only.
    #[error("'{0}' is not a valid key")]
    InvalidKvKey(String),
    /// No value is stored under the requested key.
    #[error("no value '{0}'")]
    UnknownValue(String),
    /// The derived or given process name was empty or held a control
    /// character.
    #[error("'{0}' is not a valid process name")]
    InvalidName(String),
    /// The command was blank.
    #[error("the command is empty")]
    EmptyCommand,
    /// The command could not be reassembled into a shell string.
    #[error("the command cannot be represented as a shell command")]
    InvalidCommand,
    /// A `--env` was not `KEY=VALUE`, or its name or value is not allowed.
    #[error(
        "'{0}' is not a valid environment variable; pass KEY=VALUE, where KEY has no '=' and does not start with MUSTER_"
    )]
    InvalidEnv(String),
    /// A `--log` or `--ready-log` pattern was blank, too long, or a regex
    /// that does not compile.
    #[error(transparent)]
    InvalidPattern(#[from] PatternError),
    /// A `--description` was blank.
    #[error("the description is empty; pass --no-description to clear it")]
    EmptyDescription,
    /// A `--cwd` is not an existing directory.
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    /// A folder or file path is not valid UTF-8, so it cannot be stored.
    #[error("'{}' is not valid UTF-8 and cannot be stored", .0.display())]
    UnrepresentablePath(PathBuf),
    /// Stored state could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Talking to the service failed.
    #[error("{}", client_message(.0))]
    Client(#[from] ClientError),
    /// A process of the project already has the name being added.
    #[error(
        "a process named {} already exists in {project}{hint}",
        name.shell_quoted()
    )]
    ProcessNameTaken {
        /// The name.
        name: ProcessName,
        /// The project's name.
        project: ProjectName,
        /// What to try instead, for the command that chose the name.
        hint: &'static str,
    },
    /// `muster set` was given nothing to change.
    #[error("nothing to change; pass at least one flag, as `muster set --help` lists them")]
    NothingToChange,
    /// A stack file could not be read.
    #[error("could not read {}: {source}", path.display())]
    ReadFile {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// A stack file does not hold a stack muster can import.
    #[error("{} is not a stack muster can import: {source}", path.display())]
    StackFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        #[source]
        source: ExchangeError,
    },
    /// A coordination snapshot could not be written or read back.
    #[error("{} is not a coordination snapshot muster can use: {reason}", path.display())]
    SnapshotFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// `muster export` would overwrite a file.
    #[error("{} already exists; pass --force to overwrite it", .0.display())]
    FileExists(PathBuf),
    /// The command line could not be parsed.
    #[error("{0}")]
    Usage(String),
    /// Writing the command's output failed.
    #[error("could not write the output: {0}")]
    Output(#[source] io::Error),
    /// `muster attach` was run without a terminal to take over.
    #[error(
        "muster attach needs a terminal on its input and output; `muster logs -f` follows a process without one"
    )]
    NotATerminal,
    /// The terminal could not be switched or drawn on.
    #[error("could not drive the terminal: {0}")]
    Terminal(#[source] io::Error),
    /// The command needed a question answered and had no interactive
    /// terminal to ask it on.
    #[error("cannot ask {question} without an interactive terminal; pass {argument}")]
    CannotAsk {
        /// What would have been asked.
        question: &'static str,
        /// The argument that answers it up front.
        argument: &'static str,
    },
    /// A confirmation `--yes` gives up front was needed and nobody could be
    /// asked.
    #[error("cannot ask {0} without an interactive terminal; pass --yes")]
    NeedsYes(&'static str),
    /// The current directory could not be read.
    #[error("could not read the current directory: {0}")]
    CurrentDir(#[source] io::Error),
    /// A process was to be chosen from a project that has none.
    #[error("{0} has no processes to choose from; `muster add` adds one")]
    NothingToChoose(ProjectName),
    /// A running process was to be chosen from a project that runs none.
    #[error("no process of {0} is running; `muster up` starts them")]
    NothingRunning(ProjectName),
    /// The person cancelled a question with Esc or Ctrl-C.
    #[error("cancelled; nothing was changed")]
    Cancelled,
    /// The connection to a process's holder broke while attached.
    #[error(
        "lost the connection to {}: {source}; `muster ps` shows whether it still runs",
        name.shell_quoted()
    )]
    AttachLost {
        /// The process.
        name: ProcessName,
        /// What broke.
        #[source]
        source: HolderError,
    },
}

impl From<ResolveError> for CliError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::UnknownProject { name, .. } => Self::UnknownProject(name),
            ResolveError::NotAProject { dir } => Self::NotAProject { dir },
            ResolveError::Config(error) => Self::Config(error),
        }
    }
}

impl CliError {
    /// The kind of failure, for `--json` output and the exit code.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::UnknownProject(_) | Self::NotAProject { .. } => FailureKind::UnknownProject,
            Self::Config(error) => FailureKind::from(error),
            Self::Client(error) => error.kind(),
            Self::NothingRunning(_) => FailureKind::NotRunning,
            Self::UnknownNote(_) => FailureKind::UnknownNote,
            Self::UnknownTodo(_) => FailureKind::UnknownTodo,
            Self::UnknownValue(_) => FailureKind::UnknownKey,
            Self::Output(_)
            | Self::Terminal(_)
            | Self::AttachLost { .. }
            | Self::ReadFile { .. }
            | Self::CurrentDir(_) => FailureKind::Io,
            Self::Usage(_) | Self::Cancelled | Self::NothingToChange | Self::NeedsYes(_) => {
                FailureKind::Usage
            },
            Self::NotATerminal
            | Self::CannotAsk { .. }
            | Self::NothingToChoose(_)
            | Self::InvalidNoteKey(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::HeldByOther { .. }
            | Self::InvalidLockName(_)
            | Self::InvalidKvKey(_)
            | Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidCommand
            | Self::InvalidEnv(_)
            | Self::InvalidPattern(_)
            | Self::EmptyDescription
            | Self::NotADirectory(_)
            | Self::UnrepresentablePath(_)
            | Self::ProcessNameTaken { .. }
            | Self::StackFile { .. }
            | Self::SnapshotFile { .. }
            | Self::FileExists(_) => FailureKind::InvalidInput,
        }
    }

    /// The process names a person could have meant, for `--json` output.
    pub fn candidates(&self) -> &[ProcessName] {
        match self {
            Self::Client(error) => error.candidates(),
            Self::Config(ConfigError::UnknownProcess { valid, .. }) => valid,
            _ => &[],
        }
    }

    /// The failure of adding to `project` when the store refused with
    /// `error`: a taken name says so with `hint`, anything else passes on.
    pub fn adding(error: ConfigError, project: Option<ProjectName>, hint: &'static str) -> Self {
        match (error, project) {
            (ConfigError::DuplicateProcessName { name }, Some(project)) => Self::ProcessNameTaken {
                name,
                project,
                hint,
            },
            (other, _) => Self::Config(other),
        }
    }
}

/// `error` worded for a person at a terminal: what to run about it, where
/// the client's own message leaves that out.
fn client_message(error: &ClientError) -> String {
    match error {
        ClientError::NotRunning(name) => {
            format!("{error}; `muster start {}` starts it", name.shell_quoted())
        },
        ClientError::UnexpectedAnswer(_) | ClientError::VersionMismatch { .. } => {
            format!("{error}; {STOP_SERVICE_HINT}")
        },
        ClientError::ServiceNewer { .. } => format!("{error}; {NEWER_SERVICE_HINT}"),
        ClientError::Refused { kind, .. } => format!("{error}{}", refusal_hint(*kind)),
        ClientError::Interrupted => format!("{error}, {INTERRUPTED_HINT}"),
        ClientError::CursorAhead { .. } => format!("{error}; {CURSOR_HINT}"),
        ClientError::NoStateDir
        | ClientError::ServiceUnavailable { .. }
        | ClientError::ServiceIo { .. }
        | ClientError::UnknownProcess { .. }
        | ClientError::AmbiguousProcess { .. }
        | ClientError::Config(_)
        | ClientError::Log { .. }
        | ClientError::InterruptedUnsent => error.to_string(),
    }
}

/// What to try after a refusal of `kind`, or nothing when its message says
/// enough.
fn refusal_hint(kind: ServiceErrorKind) -> String {
    match kind {
        ServiceErrorKind::Busy => format!("; {BUSY_HINT}"),
        ServiceErrorKind::InvalidRequest => format!("; {INVALID_REQUEST_HINT}"),
        ServiceErrorKind::UnknownProcess
        | ServiceErrorKind::UnknownProject
        | ServiceErrorKind::NotRunning
        | ServiceErrorKind::InvalidInput
        | ServiceErrorKind::Internal => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::cli::cli_exit::CliExit;

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// Each stored-state failure exits by its kind: the caller's mistakes
    /// as invalid input, a store that cannot be used as a failed action.
    #[test]
    fn stored_state_failures_exit_by_their_kind() {
        let exit = |error: ConfigError| CliExit::from(CliError::Config(error).kind());

        assert_eq!(
            exit(ConfigError::UnknownProject {
                root: PathBuf::from("/p")
            }),
            CliExit::InvalidInput
        );
        assert_eq!(
            exit(ConfigError::ImportConflict {
                names: vec![name("web")]
            }),
            CliExit::InvalidInput
        );
        assert_eq!(
            exit(ConfigError::Database("locked".to_string())),
            CliExit::ActionFailed
        );
        assert_eq!(
            CliExit::from(CliError::NothingToChange.kind()),
            CliExit::Usage
        );
        assert_eq!(
            CliExit::from(
                CliError::NotAProject {
                    dir: PathBuf::from("/tmp")
                }
                .kind()
            ),
            CliExit::InvalidInput
        );
    }

    /// An unknown process the store names carries the valid names as
    /// candidates, as one the service names does.
    #[test]
    fn a_stored_unknown_process_lists_candidates() {
        let error = CliError::Config(ConfigError::UnknownProcess {
            name: name("wbe"),
            valid: vec![name("web")],
        });

        assert_eq!(error.candidates(), [name("web")]);
    }

    /// A client failure gains the command a person runs about it.
    #[test]
    fn a_client_failure_names_the_command_to_run() {
        let stopped = CliError::from(ClientError::NotRunning(name("api")));
        let skewed = CliError::from(ClientError::UnexpectedAnswer("ok"));

        assert_eq!(
            stopped.to_string(),
            "'api' is not running; `muster start 'api'` starts it"
        );
        assert!(skewed.to_string().ends_with(STOP_SERVICE_HINT), "{skewed}");
        assert_eq!(stopped.kind(), FailureKind::NotRunning);
    }

    /// Only the name being added is reported as taken, with the hint of
    /// the command that chose it.
    #[test]
    fn a_taken_name_carries_its_hint() {
        let project = ProjectName::try_new("shop").unwrap();
        let taken = CliError::adding(
            ConfigError::DuplicateProcessName { name: name("api") },
            Some(project.clone()),
            RUN_NAME_HINT,
        );

        assert_eq!(
            taken.to_string(),
            "a process named 'api' already exists in shop; pass --name to choose another"
        );
        assert!(matches!(
            CliError::adding(ConfigError::NoStateDir, Some(project), ""),
            CliError::Config(ConfigError::NoStateDir)
        ));
    }
}
