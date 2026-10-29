use std::path::PathBuf;

use super::holder_run_handle::HolderRunHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose every process runs under a holder at `socket`.
pub(crate) struct HolderRunRunner {
    socket: PathBuf,
}

impl HolderRunRunner {
    /// A runner whose processes the holder at `socket` serves.
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
}

impl ProcessRunner for HolderRunRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(HolderRunHandle::new(self.socket.clone())))
    }
}
