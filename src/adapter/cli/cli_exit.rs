use strum::EnumIter;

use crate::adapter::client::FailureKind;

/// What a command's exit code says about how it ended, so a script can tell
/// a failed action from an unreachable service or a typo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub enum CliExit {
    /// It did what was asked.
    Success,
    /// A process it acted on did not end up as asked, the action failed, or
    /// the current state refused it.
    ActionFailed,
    /// The command line could not be parsed, or a question was cancelled.
    Usage,
    /// The muster service could not be started or reached, or runs a newer
    /// version.
    ServiceUnavailable,
    /// An argument or file was not usable: an unknown project or process
    /// name, or an invalid config.
    InvalidInput,
    /// A person pressed Ctrl-C while the command waited at a terminal; the
    /// shell's convention for a command ended by `SIGINT`.
    Interrupted,
}

impl CliExit {
    /// The process exit code.
    pub fn code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::ActionFailed => 1,
            Self::Usage => 2,
            Self::ServiceUnavailable => 3,
            Self::InvalidInput => 4,
            Self::Interrupted => 130,
        }
    }
}

impl From<FailureKind> for CliExit {
    fn from(kind: FailureKind) -> Self {
        match kind {
            FailureKind::ServiceUnavailable | FailureKind::VersionMismatch | FailureKind::Busy => {
                Self::ServiceUnavailable
            },
            FailureKind::UnknownProject
            | FailureKind::InvalidConfig
            | FailureKind::UnknownProcess
            | FailureKind::Ambiguous
            | FailureKind::UnknownTodo
            | FailureKind::UnknownNote
            | FailureKind::UnknownKey
            | FailureKind::InvalidInput
            | FailureKind::NotAllowed => Self::InvalidInput,
            FailureKind::NotRunning
            | FailureKind::Conflict
            | FailureKind::Io
            | FailureKind::NoWorkspace
            | FailureKind::WorkspaceFailed => Self::ActionFailed,
            FailureKind::Usage => Self::Usage,
            FailureKind::Interrupted => Self::Interrupted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Running outside a terminal workspace, or in one that fails, exits as
    /// a failed action.
    #[test]
    fn workspace_kinds_exit_1() {
        for kind in [FailureKind::NoWorkspace, FailureKind::WorkspaceFailed] {
            assert_eq!(CliExit::from(kind), CliExit::ActionFailed);
            assert_eq!(CliExit::from(kind).code(), 1);
        }
    }
}
