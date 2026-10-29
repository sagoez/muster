use std::{
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{failure_kind::FailureKind, unavailable_reason::UnavailableReason};
use crate::{
    adapter::{exchange::ExchangeError, holder::HolderError, service::ServiceErrorKind},
    domain::{
        config::ConfigError,
        value::{ProcessName, ProjectName},
    },
};

/// What a person should try after a refusal of each kind, appended to its
/// message.
const BUSY_HINT: &str = "the muster service is shutting down; run the command again";
const INVALID_REQUEST_HINT: &str = "the running service may be another muster version; `muster service stop` and run the command again";
/// Shown in place of the valid names when a project has no processes.
const NO_PROCESSES: &str = "the project has no processes";
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
    /// A key-value key was blank or whitespace-only.
    #[error("'{0}' is not a valid key")]
    InvalidKvKey(String),
    /// No value is stored under the requested key.
    #[error("no value '{0}'")]
    UnknownValue(String),
    /// The derived or given process name was empty.
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
    /// muster has no state directory to find the service in.
    #[error("cannot find muster's state directory; set HOME or XDG_STATE_HOME")]
    NoStateDir,
    /// The service could not be started, or did not answer.
    #[error("{}", unavailable(reason, log, tail))]
    ServiceUnavailable {
        /// What went wrong.
        #[source]
        reason: UnavailableReason,
        /// The service log, where the service records why it stopped.
        log: PathBuf,
        /// The last lines of that log.
        tail: Vec<String>,
    },
    /// The exchange with a running service failed.
    #[error("could not talk to the muster service at {socket}: {source}")]
    ServiceIo {
        /// The service socket.
        socket: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The service answered with a kind of answer the command cannot use.
    #[error(
        "the muster service answered with `{0}`, which this command does not expect; `muster service stop` and run the command again"
    )]
    UnexpectedAnswer(&'static str),
    /// The running service is a newer muster than this binary, which does
    /// not replace it.
    #[error(
        "the running muster service is version {theirs}{}, newer than this muster ({ours}); run the newer muster, or `muster service stop` to let this one start its own",
        binary.as_ref().map(|binary| format!(" from {}", binary.display())).unwrap_or_default()
    )]
    ServiceNewer {
        /// This binary's version.
        ours: String,
        /// The service's version.
        theirs: String,
        /// The binary the service runs.
        binary: Option<PathBuf>,
    },
    /// The command line could not be parsed.
    #[error("{0}")]
    Usage(String),
    /// A replacement service still runs another version than this binary.
    #[error(
        "the muster service runs version {theirs} after being replaced for this binary's {ours}; `muster service stop` and run the command again"
    )]
    VersionMismatch {
        /// This binary's version.
        ours: String,
        /// The service's version.
        theirs: String,
    },
    /// The project has no process by the name typed.
    #[error("no process '{name}' in {project}; {}", valid_names(valid))]
    UnknownProcess {
        /// The name as typed.
        name: String,
        /// The project's name.
        project: ProjectName,
        /// Every process name the project has.
        valid: Vec<ProcessName>,
    },
    /// The name typed matches several processes only by Unicode encoding.
    #[error(
        "'{name}' matches several processes in {project} that differ only in how their accents are encoded: {}; remove one of them",
        quoted(candidates)
    )]
    AmbiguousProcess {
        /// The name as typed.
        name: String,
        /// The project's name.
        project: ProjectName,
        /// The processes it matches.
        candidates: Vec<ProcessName>,
    },
    /// The process has nothing running to reach.
    #[error("{} is not running; `muster start {}` starts it", .0.shell_quoted(), .0.shell_quoted())]
    NotRunning(ProcessName),
    /// The service refused the request.
    #[error("{message}{}", refusal_hint(*kind))]
    Refused {
        /// Why, as a kind.
        kind: ServiceErrorKind,
        /// Why, for a person to read.
        message: String,
    },
    /// Reading or following a log failed.
    #[error("could not read the log {path}: {source}")]
    Log {
        /// The log file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
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
    /// The person pressed Ctrl-C before the command sent its request.
    #[error("cancelled; nothing was changed")]
    InterruptedUnsent,
    /// The person pressed Ctrl-C while the command waited for its processes.
    #[error(
        "interrupted; the muster service keeps settling the processes, `muster ps` shows how they end"
    )]
    Interrupted,
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

impl CliError {
    /// The kind of failure, for `--json` output and the exit code.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::NoStateDir | Self::ServiceUnavailable { .. } | Self::ServiceIo { .. } => {
                FailureKind::ServiceUnavailable
            },
            Self::UnexpectedAnswer(_)
            | Self::VersionMismatch { .. }
            | Self::ServiceNewer { .. } => FailureKind::VersionMismatch,
            Self::UnknownProject(_) | Self::NotAProject { .. } => FailureKind::UnknownProject,
            Self::Config(error) => config_kind(error),
            Self::UnknownProcess { .. } => FailureKind::UnknownProcess,
            Self::AmbiguousProcess { .. } => FailureKind::Ambiguous,
            Self::NotRunning(_) | Self::NothingRunning(_) => FailureKind::NotRunning,
            Self::Refused { kind, .. } => FailureKind::from(*kind),
            Self::Log { .. }
            | Self::Output(_)
            | Self::Terminal(_)
            | Self::AttachLost { .. }
            | Self::ReadFile { .. }
            | Self::CurrentDir(_) => FailureKind::Io,
            Self::Usage(_) | Self::Cancelled | Self::NothingToChange | Self::NeedsYes(_) => {
                FailureKind::Usage
            },
            Self::Interrupted | Self::InterruptedUnsent => FailureKind::Interrupted,
            Self::NotATerminal
            | Self::CannotAsk { .. }
            | Self::NothingToChoose(_)
            | Self::InvalidNoteKey(_)
            | Self::UnknownNote(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::UnknownTodo(_)
            | Self::InvalidKvKey(_)
            | Self::UnknownValue(_)
            | Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidCommand
            | Self::InvalidEnv(_)
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
            Self::UnknownProcess { valid, .. }
            | Self::Config(ConfigError::UnknownProcess { valid, .. }) => valid,
            Self::AmbiguousProcess { candidates, .. } => candidates,
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

/// The kind of a stored-state failure: the caller's mistake for a wrong
/// name, value or file, muster's own for a store that cannot be used.
fn config_kind(error: &ConfigError) -> FailureKind {
    match error {
        ConfigError::UnknownProject { .. } => FailureKind::UnknownProject,
        ConfigError::UnknownProcess { .. } => FailureKind::UnknownProcess,
        ConfigError::DuplicateProcessName { .. }
        | ConfigError::ImportConflict { .. }
        | ConfigError::InvalidSetting { .. }
        | ConfigError::InvalidDefaultSize { .. }
        | ConfigError::RootNotADirectory { .. } => FailureKind::InvalidInput,
        ConfigError::UnsupportedSchema { .. } => FailureKind::InvalidConfig,
        ConfigError::Write { .. }
        | ConfigError::NoStateDir
        | ConfigError::CoordinationId(_)
        | ConfigError::Database(_) => FailureKind::Io,
    }
}

/// The message of [`CliError::ServiceUnavailable`]: the reason, then the
/// log's last lines or where the log is.
fn unavailable(reason: &UnavailableReason, log: &Path, tail: &[String]) -> String {
    if tail.is_empty() {
        return format!(
            "the muster service is not available: {reason}; its log is {}",
            log.display()
        );
    }
    let lines: Vec<String> = tail.iter().map(|line| format!("  {line}")).collect();
    format!(
        "the muster service is not available: {reason}\nlast lines of {}:\n{}",
        log.display(),
        lines.join("\n")
    )
}

/// The valid names after an unknown one, each quoted so spaces show.
fn valid_names(valid: &[ProcessName]) -> String {
    if valid.is_empty() {
        return NO_PROCESSES.to_string();
    }
    format!("valid names: {}", quoted(valid))
}

/// `names` each quoted for a shell and joined by commas.
fn quoted(names: &[ProcessName]) -> String {
    names
        .iter()
        .map(ProcessName::shell_quoted)
        .collect::<Vec<_>>()
        .join(", ")
}

/// What to try after a refusal of `kind`, or nothing when its message says
/// enough.
fn refusal_hint(kind: ServiceErrorKind) -> String {
    match kind {
        ServiceErrorKind::Busy => format!("; {BUSY_HINT}"),
        ServiceErrorKind::InvalidRequest => format!("; {INVALID_REQUEST_HINT}"),
        ServiceErrorKind::UnknownProcess
        | ServiceErrorKind::UnknownProject
        | ServiceErrorKind::InvalidConfig
        | ServiceErrorKind::NotRunning
        | ServiceErrorKind::Refused
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
