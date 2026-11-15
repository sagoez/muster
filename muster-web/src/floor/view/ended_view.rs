use std::time::UNIX_EPOCH;

use getset::Getters;
use muster_protocol::service::LastExit;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// How a process's last run ended, as its card shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct EndedView {
    /// How it ended, as every surface says it: `exit 1`, `signal SIGKILL`.
    text: String,
    /// When it ended, in seconds since the Unix epoch, when the service saw it.
    at: Option<u64>,
}

impl From<&LastExit> for EndedView {
    fn from(exit: &LastExit) -> Self {
        Self::builder()
            .text(exit.outcome().to_string())
            .at(exit
                .at()
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_secs()))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use muster_domain::pty::{ExitCause, ExitOutcome};
    use muster_protocol::service::ExitReason;

    use super::*;

    /// A run's end shows its outcome and the second it ended.
    #[test]
    fn an_end_shows_its_outcome_and_when() {
        let exit = LastExit::builder()
            .outcome(ExitOutcome::Failed(ExitCause::Status(1)))
            .reason(ExitReason::Crashed)
            .at(Some(UNIX_EPOCH + Duration::from_millis(1_500)))
            .build();

        let ended = EndedView::from(&exit);

        assert_eq!(ended.text(), "exit 1");
        assert_eq!(*ended.at(), Some(1));
    }
}
