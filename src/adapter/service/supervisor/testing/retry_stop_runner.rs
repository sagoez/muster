use std::sync::Arc;

use super::{
    retry_stop_handle::RetryStopHandle, retry_stop_signals::RetryStopSignals,
    terminate_outcome::TerminateOutcome,
};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner used to verify that failed stop delivery remains retryable.
pub(crate) struct RetryStopRunner {
    pub(crate) signals: Arc<RetryStopSignals>,
    pub(crate) terminate_outcome: TerminateOutcome,
}

impl ProcessRunner for RetryStopRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(RetryStopHandle {
            signals: self.signals.clone(),
            terminate_outcome: self.terminate_outcome,
        }))
    }
}
