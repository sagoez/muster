use serde::Serialize;

use crate::{adapter::service::ServiceErrorKind, domain::config::ConfigError};

/// Why a request failed, as JSON output names it so a script or an agent can
/// act on it without parsing the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    /// The service could not be started or reached.
    ServiceUnavailable,
    /// The service speaks another version of the protocol.
    VersionMismatch,
    /// No registered project matched the name or folder the request used.
    UnknownProject,
    /// muster's stored state was written by a newer muster.
    InvalidConfig,
    /// No process has the name typed; `candidates` lists the valid ones.
    UnknownProcess,
    /// The name typed matches more than one process.
    Ambiguous,
    /// The process has nothing running to reach.
    NotRunning,
    /// The service is shutting down.
    Busy,
    /// A process the request acted on did not reach the state asked for.
    ProcessFailed,
    /// Reading a log, writing the output, driving the terminal, keeping an
    /// attached process's connection, or reading or writing stored state
    /// failed.
    Io,
    /// The request's own arguments or files were not usable.
    InvalidInput,
    /// The command line itself could not be parsed, or a question was
    /// cancelled.
    Usage,
    /// A person pressed Ctrl-C while the command waited at a terminal.
    Interrupted,
}

impl FailureKind {
    /// Whether the request, rather than muster, is at fault, so the caller
    /// can correct it: a wrong name, an unusable input or project, or a
    /// refusal of what was asked.
    pub fn is_caller_fault(self) -> bool {
        match self {
            Self::UnknownProject
            | Self::InvalidConfig
            | Self::UnknownProcess
            | Self::Ambiguous
            | Self::NotRunning
            | Self::ProcessFailed
            | Self::InvalidInput
            | Self::Usage => true,
            Self::ServiceUnavailable
            | Self::VersionMismatch
            | Self::Busy
            | Self::Io
            | Self::Interrupted => false,
        }
    }
}

impl From<ServiceErrorKind> for FailureKind {
    fn from(kind: ServiceErrorKind) -> Self {
        match kind {
            ServiceErrorKind::UnknownProcess => Self::UnknownProcess,
            ServiceErrorKind::UnknownProject => Self::UnknownProject,
            ServiceErrorKind::NotRunning => Self::NotRunning,
            ServiceErrorKind::InvalidRequest => Self::VersionMismatch,
            ServiceErrorKind::InvalidInput => Self::InvalidInput,
            ServiceErrorKind::Busy => Self::Busy,
            ServiceErrorKind::Internal => Self::Io,
        }
    }
}

impl From<&ConfigError> for FailureKind {
    /// The caller's mistake for a wrong name, value or file; muster's own
    /// for a store that cannot be used.
    fn from(error: &ConfigError) -> Self {
        match error {
            ConfigError::UnknownProject { .. } => Self::UnknownProject,
            ConfigError::UnknownProcess { .. } => Self::UnknownProcess,
            ConfigError::DuplicateProcessName { .. }
            | ConfigError::ImportConflict { .. }
            | ConfigError::InvalidSetting { .. }
            | ConfigError::InvalidDefaultSize { .. }
            | ConfigError::RootNotADirectory { .. }
            | ConfigError::RootNotUtf8 { .. } => Self::InvalidInput,
            ConfigError::UnsupportedSchema { .. } => Self::InvalidConfig,
            ConfigError::Write { .. }
            | ConfigError::NoStateDir
            | ConfigError::CoordinationId(_)
            | ConfigError::Database(_) => Self::Io,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A wrong name or input is the caller's to fix; a service that cannot
    /// be reached or speaks another version is muster's.
    #[test]
    fn a_wrong_name_is_the_callers_fault_and_an_unreachable_service_is_not() {
        assert!(FailureKind::UnknownProcess.is_caller_fault());
        assert!(FailureKind::NotRunning.is_caller_fault());
        assert!(FailureKind::InvalidInput.is_caller_fault());
        assert!(!FailureKind::ServiceUnavailable.is_caller_fault());
        assert!(!FailureKind::VersionMismatch.is_caller_fault());
        assert!(!FailureKind::Busy.is_caller_fault());
    }

    /// A stored-state failure is the caller's for a wrong name or value and
    /// muster's for a database that cannot be used.
    #[test]
    fn a_stored_state_failure_is_classified_by_its_cause() {
        let unknown = ConfigError::UnknownProject {
            root: PathBuf::from("/p"),
        };
        let broken = ConfigError::Database("locked".to_string());

        assert_eq!(FailureKind::from(&unknown), FailureKind::UnknownProject);
        assert_eq!(FailureKind::from(&broken), FailureKind::Io);
        assert!(!FailureKind::from(&broken).is_caller_fault());
    }
}
