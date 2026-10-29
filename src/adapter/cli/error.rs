use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use thiserror::Error;

use super::{failure_kind::FailureKind, unavailable_reason::UnavailableReason};
use crate::{
    adapter::{holder::HolderError, service::ServiceErrorKind},
    domain::{config::ConfigError, value::ProcessName},
};

/// What a person should try after a refusal of each kind, appended to its
/// message.
const BUSY_HINT: &str = "the muster service is shutting down; run the command again";
const INVALID_REQUEST_HINT: &str = "the running service may be another muster version; `muster service stop` and run the command again";
/// Shown in place of the valid names when a project has no processes.
const NO_PROCESSES: &str = "the project has no processes";

/// A failure while executing a CLI command.
#[derive(Debug, Error)]
pub enum CliError {
    /// No registered project matched the requested name.
    #[error("unknown project '{0}'")]
    UnknownProject(String),
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
    /// The workspace file could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The path cannot be stored in the registry (not valid UTF-8).
    #[error("'{0}' contains non-UTF-8 components and cannot be registered")]
    UnrepresentablePath(PathBuf),
    /// The folder name could not become a project name.
    #[error("cannot derive a project name from '{0}'")]
    InvalidProjectFolder(PathBuf),
    /// The folder has no workspace file to register.
    #[error("{0} not found; run `muster init` there first")]
    MissingWorkspaceFile(PathBuf),
    /// No registered project matched, listing what exists.
    #[error("unknown project '{name}'; registered: {known}")]
    UnknownProjectAmong { name: String, known: String },
    /// More than one registered project has the requested name.
    #[error("'{name}' matches {count} projects; remove is ambiguous. Registered paths: {paths}")]
    AmbiguousProject {
        /// The name that matched multiple projects.
        name: String,
        /// How many projects matched.
        count: usize,
        /// The config paths of the matching projects, joined by ", ".
        paths: String,
    },
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
    /// No `muster.yml` exists at the project path.
    #[error("no muster config at {0}; run `muster init` in that folder, or pass --config <file>")]
    NoProjectConfig(PathBuf),
    /// The project's config exists but does not load.
    #[error("{path} does not load: {message}; `muster check` explains")]
    InvalidProjectConfig {
        /// The config path.
        path: PathBuf,
        /// Why it does not load.
        message: String,
    },
    /// The project has no process by the name typed.
    #[error("no process '{name}' in {project}; {}", valid_names(valid))]
    UnknownProcess {
        /// The name as typed.
        name: String,
        /// The project's config path.
        project: PathBuf,
        /// Every process name the project has.
        valid: Vec<ProcessName>,
    },
    /// The name typed matches several processes only by Unicode encoding.
    #[error(
        "'{name}' matches several processes in {project} that differ only in how their accents are encoded: {}; rename one of them",
        quoted(candidates)
    )]
    AmbiguousProcess {
        /// The name as typed.
        name: String,
        /// The project's config path.
        project: PathBuf,
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
    /// `muster run` was given, or defaulted to, a name a process already has.
    #[error(
        "a process named {} already exists in {}; pass --name to choose another",
        name.shell_quoted(),
        project.display()
    )]
    RunNameTaken {
        /// The name.
        name: ProcessName,
        /// The project's config path.
        project: PathBuf,
    },
    /// A registered command never became known to the service.
    #[error(
        "{} was added to {}, but the muster service did not pick it up within {}; run `muster start {}`",
        name.shell_quoted(),
        project.display(),
        humantime::format_duration(*waited),
        name.shell_quoted()
    )]
    NotPickedUp {
        /// The registered process.
        name: ProcessName,
        /// The project's config path.
        project: PathBuf,
        /// How long the command waited.
        waited: Duration,
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
    /// A process was to be chosen from a project that has none.
    #[error("{0} has no processes to choose from; add some to it first")]
    NothingToChoose(PathBuf),
    /// A running process was to be chosen from a project that runs none.
    #[error("no process of {0} is running; `muster up` starts them")]
    NothingRunning(PathBuf),
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
            Self::NoProjectConfig(_)
            | Self::UnknownProject(_)
            | Self::UnknownProjectAmong { .. }
            | Self::MissingWorkspaceFile(_) => FailureKind::UnknownProject,
            Self::InvalidProjectConfig { .. } | Self::Config(_) => FailureKind::InvalidConfig,
            Self::UnknownProcess { .. } => FailureKind::UnknownProcess,
            Self::AmbiguousProcess { .. } | Self::AmbiguousProject { .. } => FailureKind::Ambiguous,
            Self::NotRunning(_) => FailureKind::NotRunning,
            Self::Refused { kind, .. } => FailureKind::from(*kind),
            Self::NotPickedUp { .. } => FailureKind::Refused,
            Self::Log { .. } | Self::Output(_) | Self::Terminal(_) | Self::AttachLost { .. } => {
                FailureKind::Io
            },
            Self::NotATerminal | Self::CannotAsk { .. } | Self::NothingToChoose(_) => {
                FailureKind::InvalidInput
            },
            Self::Usage(_) | Self::Cancelled => FailureKind::Usage,
            Self::NothingRunning(_) => FailureKind::NotRunning,
            Self::Interrupted | Self::InterruptedUnsent => FailureKind::Interrupted,
            Self::InvalidNoteKey(_)
            | Self::UnknownNote(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::UnknownTodo(_)
            | Self::InvalidKvKey(_)
            | Self::UnknownValue(_)
            | Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidCommand
            | Self::UnrepresentablePath(_)
            | Self::InvalidProjectFolder(_)
            | Self::RunNameTaken { .. } => FailureKind::InvalidInput,
        }
    }

    /// The process names a person could have meant, for `--json` output.
    pub fn candidates(&self) -> &[ProcessName] {
        match self {
            Self::UnknownProcess { valid, .. } => valid,
            Self::AmbiguousProcess { candidates, .. } => candidates,
            _ => &[],
        }
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
