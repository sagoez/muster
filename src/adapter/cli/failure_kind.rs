use serde::Serialize;

use crate::adapter::service::ServiceErrorKind;

/// Why a command failed, as `--json` output names it so a script can act on
/// it without parsing the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    /// The service could not be started or reached.
    ServiceUnavailable,
    /// The service speaks another version of the protocol.
    VersionMismatch,
    /// No registered project matched the name or folder the command used.
    UnknownProject,
    /// muster's stored state was written by a newer muster, or the service
    /// could not load a project's stack.
    InvalidConfig,
    /// No process has the name typed; `candidates` lists the valid ones.
    UnknownProcess,
    /// The name typed matches more than one project or process.
    Ambiguous,
    /// The process has nothing running to reach.
    NotRunning,
    /// The service refused the request.
    Refused,
    /// The service is shutting down.
    Busy,
    /// A process the command acted on did not reach the state asked for.
    ProcessFailed,
    /// Reading a log, writing the output, driving the terminal, keeping an
    /// attached process's connection, or the service's own reading of its
    /// state failed.
    Io,
    /// The command's own arguments or files were not usable.
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
            | Self::Refused
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
            ServiceErrorKind::InvalidConfig => Self::InvalidConfig,
            ServiceErrorKind::NotRunning => Self::NotRunning,
            ServiceErrorKind::Refused => Self::Refused,
            ServiceErrorKind::InvalidRequest => Self::VersionMismatch,
            ServiceErrorKind::Busy => Self::Busy,
            ServiceErrorKind::Internal => Self::Io,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
