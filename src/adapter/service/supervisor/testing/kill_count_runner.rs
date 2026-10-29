use std::sync::{Arc, atomic::AtomicUsize};

use super::kill_count_handle::KillCountHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose handles count how many times they are killed.
pub(crate) struct KillCountRunner {
    pub(crate) kills: Arc<AtomicUsize>,
}

impl ProcessRunner for KillCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(KillCountHandle {
            kills: self.kills.clone(),
        }))
    }
}
