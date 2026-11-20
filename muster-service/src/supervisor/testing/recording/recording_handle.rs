use std::sync::{Arc, Mutex};

use getset::Getters;
use muster_domain::pty::PtyError;
use typed_builder::TypedBuilder;

use crate::supervisor::testing::StubHandle;

/// A handle that records everything written to its PTY.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RecordingHandle {
    written: Arc<Mutex<Vec<u8>>>,
}

impl StubHandle for RecordingHandle {
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.written.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }
}
