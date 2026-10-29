use std::{
    io::{self, Read},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::protocol::HolderMessage;
use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// Reported when the holder closed the connection without an exit and no
/// longer serves its socket.
pub(super) const HOLDER_LOST: &str = "the holder stopped serving without reporting an exit";
/// How long a probe of a holder's socket waits for a dying holder's exit to
/// reset the connection its listener still queued.
const SERVING_PROBE: Duration = Duration::from_millis(500);

/// Everything the reader thread of a holder connection owns.
pub(super) struct Reader {
    /// The connection's read half.
    pub(super) stream: UnixStream,
    /// The holder's socket, probed when the connection drops unexpectedly.
    pub(super) socket: PathBuf,
    /// Where the child's screen, output and exit go.
    pub(super) sink: Arc<dyn OutputSink>,
    /// Set when the handle was dropped.
    pub(super) detached: Arc<AtomicBool>,
    /// Set when the holder reports the exit.
    pub(super) exit_held: Arc<AtomicBool>,
}

/// Forwards the holder's frames to the sink until the child exits or the
/// connection closes. A sink that is slow only delays this thread; the holder
/// resyncs a supervisor that falls behind rather than waiting for it.
///
/// The exit is forwarded, never collected: the holder keeps it until the
/// supervisor acknowledges it, so an exit lost on the way leaves the holder
/// re-adoptable. A frame that cannot be decoded is reported as unreadable
/// output and ends the reader. A connection lost without an exit is reported
/// as unreadable too, when the handle was not dropped and the holder no
/// longer serves its socket, so the supervisor's reconnect decides whether
/// the child outlived its holder; a holder that still serves has handed the
/// child to another supervisor, which is reported as nothing.
pub(super) fn spawn_reader(mut reader: Reader) {
    thread::spawn(move || {
        loop {
            match HolderMessage::read_from(&mut reader.stream) {
                Ok(HolderMessage::Snapshot {
                    repaint,
                    size,
                    attached,
                    ..
                }) => {
                    reader.sink.send(ProcessOutput::Repaint {
                        bytes: repaint,
                        size,
                    });
                    reader.sink.send(ProcessOutput::Attached(attached));
                },
                Ok(HolderMessage::Output { at, bytes }) => {
                    reader.sink.send(ProcessOutput::Placed { at, bytes });
                },
                Ok(HolderMessage::Exited(outcome)) => {
                    reader.exit_held.store(true, Ordering::SeqCst);
                    reader.sink.send(ProcessOutput::Exited(outcome));
                    return;
                },
                Ok(HolderMessage::Warning(text)) => reader.sink.send(ProcessOutput::Warning(text)),
                Ok(HolderMessage::Attached(count)) => {
                    reader.sink.send(ProcessOutput::Attached(count));
                },
                Ok(HolderMessage::Rejected(_)) => {},
                Err(error) if error.is_undecodable() => {
                    if !reader.detached.load(Ordering::SeqCst) {
                        reader
                            .sink
                            .send(ProcessOutput::Unreadable(error.to_string()));
                    }
                    return;
                },
                Err(_) => break,
            }
        }
        if !reader.detached.load(Ordering::SeqCst) && !holder_serves(&reader.socket) {
            reader
                .sink
                .send(ProcessOutput::Unreadable(HOLDER_LOST.to_string()));
        }
    });
}

/// Whether a holder still serves `socket`: it takes a connection and keeps it
/// open for [`SERVING_PROBE`]. A holder killed mid-exit may close this
/// connection before its listener, which then still queues a connect until
/// the exit resets it, so a connect alone would mistake it for serving.
fn holder_serves(socket: &Path) -> bool {
    let Ok(mut probe) = UnixStream::connect(socket) else {
        return false;
    };
    if probe.set_read_timeout(Some(SERVING_PROBE)).is_err() {
        return true;
    }
    match probe.read(&mut [0; 1]) {
        Ok(read) => read > 0,
        Err(error) => matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
        ),
    }
}
