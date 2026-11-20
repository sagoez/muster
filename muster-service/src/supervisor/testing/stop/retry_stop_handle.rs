use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use getset::Getters;
use muster_domain::{process::StopSignal, pty::PtyError};
use typed_builder::TypedBuilder;

use super::{retry_stop_signals::RetryStopSignals, terminate_outcome::TerminateOutcome};
use crate::supervisor::testing::{StubHandle, fixtures::SIGNAL_FAILURE};

/// A handle whose graceful termination follows its configured outcome and
/// whose hard kill succeeds on the second attempt.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RetryStopHandle {
    signals: Arc<RetryStopSignals>,
    terminate_outcome: TerminateOutcome,
}

impl StubHandle for RetryStopHandle {
    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates().fetch_add(1, Ordering::SeqCst);
        match self.terminate_outcome {
            TerminateOutcome::Succeeds => Ok(()),
            TerminateOutcome::Fails => Err(PtyError::Unsupported(SIGNAL_FAILURE.to_string())),
        }
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        let attempt = self.signals.kills().fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(PtyError::Unsupported(SIGNAL_FAILURE.to_string()))
        } else {
            Ok(())
        }
    }
}
