use std::sync::Arc;

use super::{stop_signal_handle::StopSignalHandle, stop_signals::StopSignals};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles record which stop mechanism was selected.
pub(crate) struct StopSignalRunner {
    pub(crate) signals: Arc<StopSignals>,
}

impl ProcessRunner for StopSignalRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(StopSignalHandle {
            signals: self.signals.clone(),
        }))
    }
}
