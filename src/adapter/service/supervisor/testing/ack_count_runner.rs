use std::sync::{Arc, atomic::AtomicUsize};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::ack_count_handle::AckCountHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles count the exits acknowledged to them.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct AckCountRunner {
    acks: Arc<AtomicUsize>,
}

impl ProcessRunner for AckCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            AckCountHandle::builder()
                .acks(Arc::clone(&self.acks))
                .build(),
        ))
    }
}
