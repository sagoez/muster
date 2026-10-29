use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use crate::{
    adapter::holder::abandon,
    domain::{
        port::{ProcessHandle, ProcessInput},
        pty::{PtyError, PtySize, ScreenSource},
        value::Pid,
    },
};

/// A handle whose holder, when it has one, serves at `socket` but never
/// answers, counting the kills it is sent.
pub(super) struct StalledHandle {
    pub(super) socket: Option<PathBuf>,
    pub(super) kills: Arc<AtomicUsize>,
}

impl ProcessHandle for StalledHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        self.socket
            .clone()
            .map_or(ScreenSource::Kept, ScreenSource::Served)
    }

    fn abandon(mut self: Box<Self>) {
        match self.socket.take() {
            Some(socket) => {
                thread::spawn(move || abandon(&socket));
            },
            None => {
                let _ = self.kill();
            },
        }
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
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
