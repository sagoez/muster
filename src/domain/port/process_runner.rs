use std::sync::Arc;

use super::{OutputSink, ProcessHandle};
use crate::domain::pty::{PtyError, SpawnRequest};

/// Driven port: spawns a process under a PTY and streams its output to a sink.
pub trait ProcessRunner {
    /// Spawns `request`'s command, delivering output to `sink`, and returns a
    /// handle for interacting with the running process.
    ///
    /// # Errors
    /// Returns a `PtyError` if the process cannot be spawned.
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError>;

    /// This runner, shareable with a worker thread, when a spawn may block
    /// long enough that the caller must not wait for it on its own loop.
    /// Defaults to none: spawns are quick and run inline.
    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        None
    }
}
