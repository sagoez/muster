use std::sync::Arc;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{stop_signal_handle::StopSignalHandle, stop_signals::StopSignals};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles record which stop mechanism was selected.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct StopSignalRunner {
    signals: Arc<StopSignals>,
}

impl ProcessRunner for StopSignalRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            StopSignalHandle::builder()
                .signals(self.signals.clone())
                .build(),
        ))
    }
}
