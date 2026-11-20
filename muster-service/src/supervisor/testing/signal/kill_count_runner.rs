use std::sync::{Arc, atomic::AtomicUsize};

use getset::Getters;
use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};
use typed_builder::TypedBuilder;

use super::kill_count_handle::KillCountHandle;
use crate::supervisor::testing::StubHandle;

/// A runner whose handles count how many times they are killed.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct KillCountRunner {
    kills: Arc<AtomicUsize>,
}

impl ProcessRunner for KillCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(KillCountHandle::builder()
            .kills(self.kills.clone())
            .build()
            .boxed())
    }
}
