use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use getset::{Getters, MutGetters};
use typed_builder::TypedBuilder;

use crate::{
    adapter::holder::{ServedScreen, abandon},
    domain::{
        port::{ProcessHandle, ProcessInput},
        process::StopSignal,
        pty::{PtyError, PtySize, ScreenSource},
        value::{OutputCursor, Pid},
    },
};

/// A handle whose holder, when it has one, serves at `socket` but never
/// answers, counting the kills it is sent.
#[derive(Getters, MutGetters, TypedBuilder)]
pub(super) struct StalledHandle {
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    socket: Option<PathBuf>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    kills: Arc<AtomicUsize>,
}

impl ProcessHandle for StalledHandle {
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn screen_size(&self) -> Option<PtySize> {
        None
    }

    fn attached_clients(&self) -> u32 {
        0
    }

    fn started_at(&self) -> Option<SystemTime> {
        None
    }

    fn run_start(&self) -> Option<OutputCursor> {
        None
    }

    fn output_end(&self) -> Option<OutputCursor> {
        None
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.kill()
    }

    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        self.socket.clone().map_or(ScreenSource::Kept, |socket| {
            ScreenSource::Served(Arc::new(ServedScreen::new(socket)))
        })
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
