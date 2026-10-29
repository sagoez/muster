use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::pty::{ExitCause, ExitOutcome, SignalName};

/// How a process last ended, as `--json` shows it: its exit status, or the
/// signal that ended it. Both are absent when the end was seen without
/// either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ExitView {
    /// The exit status; 0 is success.
    status: Option<u32>,
    /// The signal that ended it, by its lowercase POSIX name (`sigterm`).
    signal: Option<SignalName>,
}

impl From<&ExitOutcome> for ExitView {
    fn from(outcome: &ExitOutcome) -> Self {
        let (status, signal) = match outcome {
            ExitOutcome::Succeeded => (Some(0), None),
            ExitOutcome::Failed(ExitCause::Status(status)) => (Some(*status), None),
            ExitOutcome::Failed(ExitCause::Signal(signal)) => (None, Some(signal.clone())),
            ExitOutcome::Failed(ExitCause::Unknown) => (None, None),
        };
        Self { status, signal }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signal is shown by its lowercase POSIX name.
    #[test]
    fn a_signal_is_shown_by_its_posix_name() {
        let outcome =
            ExitOutcome::Failed(ExitCause::Signal(SignalName::try_new("SIGKILL").unwrap()));

        assert_eq!(
            serde_json::to_value(ExitView::from(&outcome)).unwrap(),
            serde_json::json!({ "status": null, "signal": "sigkill" })
        );
    }
}
