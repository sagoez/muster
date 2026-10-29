use std::sync::Arc;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    retry_stop_handle::RetryStopHandle, retry_stop_signals::RetryStopSignals,
    terminate_outcome::TerminateOutcome,
};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner used to verify that failed stop delivery remains retryable.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RetryStopRunner {
    signals: Arc<RetryStopSignals>,
    terminate_outcome: TerminateOutcome,
}

impl ProcessRunner for RetryStopRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            RetryStopHandle::builder()
                .signals(self.signals.clone())
                .terminate_outcome(self.terminate_outcome)
                .build(),
        ))
    }
}
