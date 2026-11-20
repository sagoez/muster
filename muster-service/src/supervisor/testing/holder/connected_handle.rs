use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use muster_domain::{pty::PtyError, value::OutputCursor};

use crate::supervisor::testing::{StubHandle, fixtures::FIRST_SCREEN};

/// A handle that connected to a screen already showing [`FIRST_SCREEN`],
/// which its log holds too, counting the kills it is sent.
pub(crate) struct ConnectedHandle(Arc<AtomicUsize>);

impl ConnectedHandle {
    /// A handle counting its kills into `kills`.
    pub(crate) fn new(kills: Arc<AtomicUsize>) -> Self {
        Self(kills)
    }
}

impl StubHandle for ConnectedHandle {
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        Some(FIRST_SCREEN.to_vec())
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        Some((OutputCursor::new(0), FIRST_SCREEN.to_vec()))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
