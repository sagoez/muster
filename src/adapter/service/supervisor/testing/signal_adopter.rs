use std::sync::Arc;

use super::{stop_signal_handle::StopSignalHandle, stop_signals::StopSignals};
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::Pid,
    },
};

/// An adopter whose every holder answers with a handle recording the stop
/// signals it is sent into the shared count.
pub(crate) struct SignalAdopter(pub(crate) Arc<StopSignals>);

impl HolderAdopter for SignalAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(StopSignalHandle {
            signals: Arc::clone(&self.0),
        }))
    }
}
