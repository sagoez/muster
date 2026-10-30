use std::{
    io,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use super::{client_message::ClientMessage, detachable_sink::DetachableSink, error::HolderError};
use crate::domain::{
    port::ProcessInput,
    pty::{ProcessOutput, PtyError},
};

/// Reported when a writer panicked mid-frame, leaving the connection unusable.
const CONNECTION_POISONED: &str = "holder connection writer is poisoned";
/// Reported when the holder took no write within
/// [`HOLDER_WRITE_TIMEOUT`](super::handle::HOLDER_WRITE_TIMEOUT).
pub(super) const WRITE_TIMED_OUT: &str = "the holder stopped reading its supervisor connection";
/// Reported for a write on a connection an earlier timeout broke.
const WRITER_BROKEN: &str = "the holder connection broke on an earlier write";

/// The write half of a holder connection, writing one whole frame at a time.
/// A write the holder does not take within
/// [`HOLDER_WRITE_TIMEOUT`](super::handle::HOLDER_WRITE_TIMEOUT) fails
/// rather than stall its caller, and breaks the connection: the frame may be
/// cut short, so nothing more is written, and the broken stream is reported
/// as unreadable output so the supervisor reconnects.
#[derive(Clone)]
pub(super) struct SharedWriter {
    /// The connection's write half.
    pub(super) stream: Arc<Mutex<UnixStream>>,
    /// Set once a write timed out, leaving the stream mid-frame.
    pub(super) broken: Arc<AtomicBool>,
    /// Where the broken stream is reported, until the handle is dropped.
    pub(super) sink: Arc<DetachableSink>,
}

impl SharedWriter {
    /// Writes `message` as one frame.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the write fails or times out, the
    /// connection is already broken, or a writer panicked.
    pub(super) fn send(&self, message: ClientMessage) -> Result<(), PtyError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(PtyError::System(WRITER_BROKEN.to_string()));
        }
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| PtyError::System(CONNECTION_POISONED.to_string()))?;
        match message.write_to(&mut *stream) {
            Err(HolderError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                self.break_connection();
                Err(PtyError::System(WRITE_TIMED_OUT.to_string()))
            },
            written => Ok(written?),
        }
    }

    /// Marks the connection broken and reports it, off the caller's thread,
    /// since the sink may be the very queue the caller drains.
    fn break_connection(&self) {
        if self.broken.swap(true, Ordering::SeqCst) || self.sink.is_detached() {
            return;
        }
        let sink = Arc::clone(&self.sink);
        thread::spawn(move || sink.send(ProcessOutput::Unreadable(WRITE_TIMED_OUT.to_string())));
    }
}

impl ProcessInput for SharedWriter {
    /// Sends `bytes` for the holder to queue to the child.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder connection fails.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.send(ClientMessage::Input(bytes.to_vec()))
    }
}
