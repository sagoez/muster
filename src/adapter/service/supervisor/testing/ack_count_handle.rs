use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A handle that counts the exits acknowledged to it.
pub(crate) struct AckCountHandle {
    pub(crate) acks: Arc<AtomicUsize>,
}

impl ProcessHandle for AckCountHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        self.acks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}
