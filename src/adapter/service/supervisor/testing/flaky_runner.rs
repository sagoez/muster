use std::sync::atomic::{AtomicUsize, Ordering};

use super::fake_handle::FakeHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner that succeeds once, then fails every subsequent spawn.
pub(crate) struct FlakyRunner {
    pub(crate) spawns: AtomicUsize,
}

impl ProcessRunner for FlakyRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if self.spawns.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Box::new(FakeHandle))
        } else {
            Err(PtyError::System("spawn failed".to_string()))
        }
    }
}
