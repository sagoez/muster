use std::sync::{Arc, atomic::AtomicUsize};

use super::ack_count_handle::AckCountHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles count the exits acknowledged to them.
pub(crate) struct AckCountRunner {
    pub(crate) acks: Arc<AtomicUsize>,
}

impl ProcessRunner for AckCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(AckCountHandle {
            acks: Arc::clone(&self.acks),
        }))
    }
}
