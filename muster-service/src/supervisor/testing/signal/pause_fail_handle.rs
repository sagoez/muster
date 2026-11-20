use muster_domain::pty::PtyError;

use crate::supervisor::testing::{StubHandle, fixtures::SIGNAL_FAILURE};

/// A handle whose pause and resume always fail.
pub(crate) struct PauseFailHandle;

impl StubHandle for PauseFailHandle {
    fn pause(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(SIGNAL_FAILURE.to_string()))
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(SIGNAL_FAILURE.to_string()))
    }
}
