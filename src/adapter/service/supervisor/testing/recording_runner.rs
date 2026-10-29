use std::sync::{Arc, Mutex};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::recording_handle::RecordingHandle;
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{PtyError, SpawnRequest},
};

/// A runner whose spawned processes record their input.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RecordingRunner {
    written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessRunner for RecordingRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            RecordingHandle::builder()
                .written(self.written.clone())
                .build(),
        ))
    }
}
