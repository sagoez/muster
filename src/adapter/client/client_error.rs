use std::{
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{failure_kind::FailureKind, unavailable_reason::UnavailableReason};
use crate::{
    adapter::{no_state_dir::NoStateDir, service::ServiceErrorKind},
    domain::{
        config::ConfigError,
        terminal_workspace::WorkspaceError,
        value::{OutputCursor, ProcessName, ProjectName},
    },
};

/// Shown in place of the valid names when a project has no processes.
const NO_PROCESSES: &str = "the project has no processes";
/// What an older service is called when it is too old to say its version.
const TOO_OLD_TO_SAY: &str = "a version too old to say which";

/// A failure talking to the muster service or reading what it reports. Its
/// messages name no command or tool; the CLI and the MCP server each add
/// what their caller does about it.
#[derive(Debug, Error)]
pub enum ClientError {
    /// muster has no state directory to find the service in.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// A socket the service or a holder would bind does not fit in a Unix
    /// socket address, so no service could listen there.
    #[error(
        "the socket path {} is too long for a Unix socket; set {variable} to a shorter folder",
        socket.display()
    )]
    SocketPathTooLong {
        /// The socket path that does not fit.
        socket: PathBuf,
        /// The variable whose folder holds it.
        variable: &'static str,
    },
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
    #[error("could not talk to the muster service at {}: {source}", socket.display())]
    ServiceIo {
        /// The service socket.
        socket: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The service answered with a kind of answer the client cannot use.
    #[error("the muster service answered with `{0}`, which this muster does not expect")]
    UnexpectedAnswer(&'static str),
    /// The running service is a newer muster than this binary, which does
    /// not replace it.
    #[error(
        "the running muster service is version {theirs}{}, newer than this muster ({ours})",
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
    /// The running service is an older muster than this binary, found by a
    /// call that never replaces one, so it was left running.
    #[error(
        "the running muster service is {}, older than this muster ({ours})",
        theirs.as_ref().map_or_else(
            || TOO_OLD_TO_SAY.to_string(),
            |theirs| format!("version {theirs}")
        )
    )]
    ServiceOlder {
        /// This binary's version.
        ours: String,
        /// The service's version, when it is new enough to say.
        theirs: Option<String>,
    },
    /// A replacement service still runs another version than this binary.
    #[error(
        "the muster service runs version {theirs} after being replaced for this binary's {ours}"
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
    #[error("{} is not running", .0.shell_quoted())]
    NotRunning(ProcessName),
    /// The service refused the request.
    #[error("{message}")]
    Refused {
        /// Why, as a kind.
        kind: ServiceErrorKind,
        /// Why, for a person to read.
        message: String,
    },
    /// The service does not know the project, or stored state failed.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Reading a log failed.
    #[error("could not read the log {}: {source}", path.display())]
    Log {
        /// The log file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// A log read was given a cursor past the end of the process's output.
    #[error("cursor {cursor} is past the end of the output, {end}")]
    CursorAhead {
        /// The cursor given.
        cursor: OutputCursor,
        /// The end of the output.
        end: OutputCursor,
    },
    /// The caller ended the wait before its request was sent.
    #[error("cancelled; nothing was changed")]
    InterruptedUnsent,
    /// The caller ended the wait for its processes to settle.
    #[error("interrupted; the muster service keeps settling the processes")]
    Interrupted,
    /// The caller does not run inside a terminal workspace to open a pane in.
    #[error("not inside a terminal workspace")]
    NoWorkspace,
    /// The terminal workspace the caller runs in did not open a pane.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// The muster binary a new pane would run to attach no longer exists,
    /// as after an upgrade replaced it, so no pane was opened.
    #[error(
        "the muster binary {} no longer exists, so no pane was opened to run it",
        .0.display()
    )]
    BinaryGone(PathBuf),
}

impl ClientError {
    /// The kind of failure, for JSON output, the exit code and whether the
    /// caller is at fault.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::NoStateDir(_)
            | Self::SocketPathTooLong { .. }
            | Self::ServiceUnavailable { .. }
            | Self::ServiceIo { .. } => FailureKind::ServiceUnavailable,
            Self::UnexpectedAnswer(_)
            | Self::VersionMismatch { .. }
            | Self::ServiceNewer { .. }
            | Self::ServiceOlder { .. } => FailureKind::VersionMismatch,
            Self::UnknownProcess { .. } => FailureKind::UnknownProcess,
            Self::AmbiguousProcess { .. } => FailureKind::Ambiguous,
            Self::NotRunning(_) => FailureKind::NotRunning,
            Self::Refused { kind, .. } => FailureKind::from(*kind),
            Self::Config(error) => FailureKind::from(error),
            Self::Log { .. } => FailureKind::Io,
            Self::CursorAhead { .. } => FailureKind::InvalidInput,
            Self::Interrupted | Self::InterruptedUnsent => FailureKind::Interrupted,
            Self::NoWorkspace => FailureKind::NoWorkspace,
            Self::Workspace(_) | Self::BinaryGone(_) => FailureKind::WorkspaceFailed,
        }
    }

    /// The process names the caller could have meant.
    pub fn candidates(&self) -> &[ProcessName] {
        match self {
            Self::UnknownProcess { valid, .. }
            | Self::Config(ConfigError::UnknownProcess { valid, .. }) => valid,
            Self::AmbiguousProcess { candidates, .. } => candidates,
            Self::NoStateDir(_)
            | Self::SocketPathTooLong { .. }
            | Self::ServiceUnavailable { .. }
            | Self::ServiceIo { .. }
            | Self::UnexpectedAnswer(_)
            | Self::ServiceNewer { .. }
            | Self::ServiceOlder { .. }
            | Self::VersionMismatch { .. }
            | Self::NotRunning(_)
            | Self::Refused { .. }
            | Self::Config(_)
            | Self::Log { .. }
            | Self::CursorAhead { .. }
            | Self::InterruptedUnsent
            | Self::Interrupted
            | Self::NoWorkspace
            | Self::Workspace(_)
            | Self::BinaryGone(_) => &[],
        }
    }
}

/// The message of [`ClientError::ServiceUnavailable`]: the reason, then the
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The process name `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// An unknown process the store names carries the valid names as
    /// candidates, as one the service names does.
    #[test]
    fn an_unknown_process_lists_candidates_whoever_names_it() {
        let stored = ClientError::Config(ConfigError::UnknownProcess {
            name: name("wbe"),
            valid: vec![name("web")],
        });
        let listed = ClientError::UnknownProcess {
            name: "wbe".to_string(),
            project: ProjectName::try_new("shop").unwrap(),
            valid: vec![name("web")],
        };

        assert_eq!(stored.candidates(), [name("web")]);
        assert_eq!(listed.candidates(), [name("web")]);
        assert_eq!(
            listed.to_string(),
            "no process 'wbe' in shop; valid names: 'web'"
        );
    }

    /// A message names no command a caller would run.
    #[test]
    fn messages_name_no_command() {
        let errors = [
            ClientError::NotRunning(name("api")),
            ClientError::UnexpectedAnswer("ok"),
            ClientError::ServiceNewer {
                ours: "0.9.1".to_string(),
                theirs: "9.0.0".to_string(),
                binary: None,
            },
            ClientError::ServiceOlder {
                ours: "0.9.1".to_string(),
                theirs: Some("0.9.0".to_string()),
            },
            ClientError::VersionMismatch {
                ours: "0.9.1".to_string(),
                theirs: "0.9.0".to_string(),
            },
            ClientError::Interrupted,
            ClientError::NoWorkspace,
        ];
        for error in errors {
            let message = error.to_string();
            assert!(!message.contains("`muster"), "{message}");
        }
    }
}
