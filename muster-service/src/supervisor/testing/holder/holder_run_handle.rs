use std::{path::PathBuf, sync::Arc};

use muster_domain::pty::ScreenSource;
use muster_protocol::holder::ServedScreen;

use crate::supervisor::testing::StubHandle;

/// A handle to a process a holder at `socket` runs.
pub(crate) struct HolderRunHandle {
    socket: PathBuf,
}

impl HolderRunHandle {
    /// A handle whose screen the holder at `socket` serves.
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
}

impl StubHandle for HolderRunHandle {
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Served(Arc::new(ServedScreen::new(self.socket.clone())))
    }
}
