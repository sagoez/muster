use std::sync::Arc;

use super::{stop_signal_handle::StopSignalHandle, stop_signals::StopSignals};
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid},
    },
};

/// An adopter whose every holder answers with a handle recording the stop
/// signals it is sent into the shared count.
pub(crate) struct SignalAdopter(Arc<StopSignals>);

impl SignalAdopter {
    /// An adopter recording into `signals`.
    pub(crate) fn new(signals: Arc<StopSignals>) -> Self {
        Self(signals)
    }
}

impl HolderAdopter for SignalAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn serving(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            StopSignalHandle::builder()
                .signals(Arc::clone(&self.0))
                .build(),
        ))
    }
}
