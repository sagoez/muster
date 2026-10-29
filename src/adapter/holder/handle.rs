use std::{
    io::{self, Read},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use super::{
    client::greet,
    error::HolderError,
    protocol::{ClientMessage, ClientRole, HolderControl, HolderMessage},
    runner::abandon,
};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{ProcessOutput, PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// Reported when a writer panicked mid-frame, leaving the connection unusable.
const CONNECTION_POISONED: &str = "holder connection writer is poisoned";
/// Longest one write to the holder may wait for it to read, so a holder that
/// stops reading fails the request instead of stalling the service.
const HOLDER_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Reported when the holder took no write within [`HOLDER_WRITE_TIMEOUT`].
const WRITE_TIMED_OUT: &str = "the holder stopped reading its supervisor connection";
/// Reported for a write on a connection an earlier timeout broke.
const WRITER_BROKEN: &str = "the holder connection broke on an earlier write";
/// Reported when the holder closed the connection without an exit and no
/// longer serves its socket.
const HOLDER_LOST: &str = "the holder stopped serving without reporting an exit";
/// How long a probe of a holder's socket waits for a dying holder's exit to
/// reset the connection its listener still queued.
const SERVING_PROBE: Duration = Duration::from_millis(500);

/// The service's supervisor connection to one holder, driving its child.
///
/// The screen at connect time is kept for [`ProcessHandle::take_initial_screen`].
/// A dedicated thread then reads the holder's frames and forwards them to the
/// output sink, blocking while the sink is full so the holder's own backlog
/// and resync take over: output, a resync screen as a chunk that repaints from
/// a reset, then the exit. The holder keeps the exit until
/// [`ProcessHandle::acknowledge_exit`] collects it, so an exit the supervisor
/// never handled survives for a later adoption. Input is queued by the holder
/// for the child while controls apply at once, so input sent before a signal
/// is not guaranteed to reach the child first. Dropping the handle disconnects
/// and leaves the child running.
pub struct HolderHandle {
    /// The connection's write half, shared with the input writers.
    writer: SharedWriter,
    /// The connection, kept outside the writer's lock so a drop can always
    /// close it, even while a write is blocked.
    closer: UnixStream,
    /// The supervised child's process id.
    child_pid: Pid,
    /// The socket the holder serves.
    socket: PathBuf,
    /// The screen the holder sent when this handle connected, until taken.
    initial_screen: Option<Vec<u8>>,
    /// The PTY size that screen was rendered at.
    screen_size: PtySize,
    /// The writable attach clients connected when this handle connected.
    attached: u32,
    /// When the child started, when known; otherwise it is taken to have
    /// started as this handle connected.
    started_at: Option<SystemTime>,
    /// Where the child's run began in its output stream.
    run_start: OutputCursor,
    /// Set when this handle is dropped, so the reader reports no exit.
    detached: Arc<AtomicBool>,
    /// Set once the holder reported the exit, which it then holds until
    /// collected.
    exit_held: Arc<AtomicBool>,
}

impl HolderHandle {
    /// Introduces `stream` as the supervisor of the holder at `socket`, keeps
    /// its screen and starts the reader forwarding to `sink`. Waits at most
    /// `timeout` for the screen.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the holder does not send its screen or
    /// the connection cannot be shared with the reader.
    pub(super) fn supervise(
        mut stream: UnixStream,
        socket: &Path,
        sink: Box<dyn OutputSink>,
        timeout: Duration,
    ) -> Result<Self, HolderError> {
        let greeting = greet(&mut stream, socket, ClientRole::Supervisor, timeout)?;
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(HOLDER_WRITE_TIMEOUT))?;
        let sink: Arc<dyn OutputSink> = Arc::from(sink);
        let closer = stream.try_clone()?;
        let detached = Arc::new(AtomicBool::new(false));
        let exit_held = Arc::new(AtomicBool::new(false));
        let writer = SharedWriter {
            stream: Arc::new(Mutex::new(stream.try_clone()?)),
            broken: Arc::new(AtomicBool::new(false)),
            sink: Arc::clone(&sink),
            detached: Arc::clone(&detached),
        };
        spawn_reader(Reader {
            stream,
            socket: socket.to_path_buf(),
            sink,
            detached: Arc::clone(&detached),
            exit_held: Arc::clone(&exit_held),
        });
        Ok(Self {
            writer,
            closer,
            child_pid: greeting.pid(),
            socket: socket.to_path_buf(),
            screen_size: greeting.size(),
            attached: greeting.attached(),
            started_at: None,
            run_start: greeting.run_start(),
            initial_screen: Some(greeting.into_repaint()),
            detached,
            exit_held,
        })
    }

    /// Records that the child started at `started_at`, as its holder record
    /// says, so an adopted process keeps its uptime.
    pub(super) fn with_started_at(mut self, started_at: SystemTime) -> Self {
        self.started_at = Some(started_at);
        self
    }

    /// Sends a control request to the holder.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder connection fails.
    fn control(&mut self, control: HolderControl) -> Result<(), PtyError> {
        self.writer.send(ClientMessage::Control(control))
    }
}

impl ProcessHandle for HolderHandle {
    fn process_id(&self) -> Option<Pid> {
        Some(self.child_pid)
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        Some(Box::new(self.writer.clone()))
    }

    /// The holder's own emulator and keyboard tracker answer the child.
    fn answers_terminal_queries(&self) -> bool {
        true
    }

    /// The holder serves the screen it renders on its socket.
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Served(self.socket.clone())
    }

    /// Disconnects, then reaches the holder afresh off the caller's thread,
    /// kills its child and collects the exit, so the holder finishes.
    fn abandon(self: Box<Self>) {
        let socket = self.socket.clone();
        drop(self);
        thread::spawn(move || abandon(&socket));
    }

    /// The holder's screen at connect time, a repaint from a reset. It is kept
    /// here rather than sent to the sink, so the caller paints it in order
    /// without pushing into its own output channel.
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        self.initial_screen.take()
    }

    fn screen_size(&self) -> Option<PtySize> {
        Some(self.screen_size)
    }

    fn attached_clients(&self) -> u32 {
        self.attached
    }

    fn started_at(&self) -> Option<SystemTime> {
        self.started_at
    }

    fn run_start(&self) -> Option<OutputCursor> {
        Some(self.run_start)
    }

    /// Collects the exit the holder is keeping, letting it finish. Nothing is
    /// sent for an exit the holder never reported, such as a lost connection.
    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        if self.exit_held.load(Ordering::SeqCst) {
            self.writer.send(ClientMessage::CollectExit)?;
        }
        Ok(())
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.writer.write_input(bytes)
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.writer.send(ClientMessage::Resize(size))
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        self.control(HolderControl::Pause)
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        self.control(HolderControl::Resume)
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        self.control(HolderControl::Terminate { signal, grace })
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.control(HolderControl::Kill)
    }
}

impl Drop for HolderHandle {
    /// Disconnects from the holder, which keeps the child running for a later
    /// adoption, and ends the reader without reporting an exit.
    fn drop(&mut self) {
        self.detached.store(true, Ordering::SeqCst);
        let _ = self.closer.shutdown(Shutdown::Both);
    }
}

/// The write half of a holder connection, writing one whole frame at a time.
/// A write the holder does not take within [`HOLDER_WRITE_TIMEOUT`] fails
/// rather than stall its caller, and breaks the connection: the frame may be
/// cut short, so nothing more is written, and the broken stream is reported
/// as unreadable output so the supervisor reconnects.
#[derive(Clone)]
struct SharedWriter {
    /// The connection's write half.
    stream: Arc<Mutex<UnixStream>>,
    /// Set once a write timed out, leaving the stream mid-frame.
    broken: Arc<AtomicBool>,
    /// Where the broken stream is reported.
    sink: Arc<dyn OutputSink>,
    /// Set when the handle was dropped, so nothing is reported.
    detached: Arc<AtomicBool>,
}

impl SharedWriter {
    /// Writes `message` as one frame.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the write fails or times out, the
    /// connection is already broken, or a writer panicked.
    fn send(&self, message: ClientMessage) -> Result<(), PtyError> {
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
        if self.broken.swap(true, Ordering::SeqCst) || self.detached.load(Ordering::SeqCst) {
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

/// Everything the reader thread owns.
struct Reader {
    /// The connection's read half.
    stream: UnixStream,
    /// The holder's socket, probed when the connection drops unexpectedly.
    socket: PathBuf,
    /// Where the child's screen, output and exit go.
    sink: Arc<dyn OutputSink>,
    /// Set when the handle was dropped.
    detached: Arc<AtomicBool>,
    /// Set when the holder reports the exit.
    exit_held: Arc<AtomicBool>,
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
fn spawn_reader(mut reader: Reader) {
    thread::spawn(move || {
        loop {
            match HolderMessage::read_from(&mut reader.stream) {
                Ok(HolderMessage::Snapshot {
                    repaint, attached, ..
                }) => {
                    reader.sink.send(ProcessOutput::Repaint(repaint));
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

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        path::Path,
        time::{Duration, Instant},
    };

    use crossbeam_channel::{RecvTimeoutError, Sender, unbounded};

    use super::*;

    /// Bound on every wait in these tests; reaching it fails the test.
    const WAIT: Duration = Duration::from_secs(10);
    /// Short root for test sockets: macOS's long temp dir breaches `sun_path`.
    const SHORT_TEMP_ROOT: &str = "/tmp";
    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "mh-";
    /// A socket path nothing serves, so a lost connection finds no holder.
    const UNSERVED_SOCKET: &str = "/nonexistent/holder.sock";
    /// PTY rows of the fake holder's screen.
    const ROWS: u16 = 24;
    /// PTY cols of the fake holder's screen.
    const COLS: u16 = 80;
    /// The child pid the fake holder reports.
    const CHILD_PID: u32 = 7;
    /// A frame whose tag no holder message carries.
    const CORRUPT_FRAME: [u8; 5] = [0xFF, 0, 0, 0, 0];
    /// The screen the fake holder greets with.
    const SCREEN: &[u8] = b"screen";
    /// How long a reader that should report nothing is watched.
    const QUIET: Duration = Duration::from_millis(300);
    /// How long a dying holder's listener outlives its supervisor connection,
    /// well within [`SERVING_PROBE`].
    const LISTENER_LINGER: Duration = Duration::from_millis(100);
    /// Input sent at once to a holder that never reads, far past what the
    /// socket buffers.
    const FLOOD_BYTES: usize = 4 * 1024 * 1024;

    /// Forwards a holder handle's output into a channel.
    struct ChannelSink(Sender<ProcessOutput>);

    impl OutputSink for ChannelSink {
        fn send(&self, output: ProcessOutput) {
            let _ = self.0.send(output);
        }
    }

    /// The service end of a connection whose other end already holds the
    /// fake holder's screen, and that other end.
    fn greeted_pair() -> (UnixStream, UnixStream) {
        let (service, mut holder) = UnixStream::pair().unwrap();
        HolderMessage::Snapshot {
            repaint: SCREEN.to_vec(),
            size: PtySize::try_new(ROWS, COLS).unwrap(),
            pid: Pid::try_new(CHILD_PID).unwrap(),
            attached: 0,
            run_start: OutputCursor::new(0),
        }
        .write_to(&mut holder)
        .unwrap();
        (service, holder)
    }

    /// A frame the reader cannot decode is reported as unreadable output,
    /// not mistaken for an exit or swallowed like an eviction.
    #[test]
    fn a_frame_that_cannot_be_decoded_is_reported_as_unreadable_output() {
        let (service, mut holder) = greeted_pair();
        holder.write_all(&CORRUPT_FRAME).unwrap();
        let (sender, outputs) = unbounded();

        let mut handle = HolderHandle::supervise(
            service,
            Path::new(UNSERVED_SOCKET),
            Box::new(ChannelSink(sender)),
            WAIT,
        )
        .unwrap();

        assert_eq!(handle.take_initial_screen(), Some(SCREEN.to_vec()));
        assert!(matches!(
            outputs.recv_timeout(WAIT),
            Ok(ProcessOutput::Unreadable(_))
        ));
        assert_eq!(
            outputs.recv_timeout(QUIET),
            Err(RecvTimeoutError::Timeout),
            "the reader stops after reporting it"
        );
    }

    /// A connection lost while the holder no longer serves its socket is
    /// reported as unreadable, never as an exit: the holder is gone but its
    /// child may still run, which only a reconnect can tell.
    #[test]
    fn a_lost_holder_is_reported_as_unreadable_not_as_an_exit() {
        let (service, holder) = greeted_pair();
        let (sender, outputs) = unbounded();
        let _handle = HolderHandle::supervise(
            service,
            Path::new(UNSERVED_SOCKET),
            Box::new(ChannelSink(sender)),
            WAIT,
        )
        .unwrap();

        holder.shutdown(Shutdown::Both).unwrap();

        assert!(matches!(
            outputs.recv_timeout(WAIT),
            Ok(ProcessOutput::Unreadable(reason)) if reason == HOLDER_LOST
        ));
    }

    /// A holder killed mid-exit, whose connection closed before its listener,
    /// is still reported lost once the exit closes the listener too, though a
    /// connect probe reached the listener first.
    #[test]
    fn a_holder_whose_listener_outlives_the_connection_briefly_is_lost() {
        let scratch = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let socket = scratch.path().join("holder.sock");
        let dying = UnixListener::bind(&socket).unwrap();
        let (service, holder) = greeted_pair();
        let (sender, outputs) = unbounded();
        let _handle =
            HolderHandle::supervise(service, &socket, Box::new(ChannelSink(sender)), WAIT).unwrap();

        holder.shutdown(Shutdown::Both).unwrap();
        thread::sleep(LISTENER_LINGER);
        drop(dying);

        assert!(matches!(
            outputs.recv_timeout(WAIT),
            Ok(ProcessOutput::Unreadable(reason)) if reason == HOLDER_LOST
        ));
    }

    /// A supervisor evicted by a holder that still serves its socket reports
    /// nothing: the process lives on under its new supervisor.
    #[test]
    fn an_evicted_supervisor_reports_nothing() {
        let scratch = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let socket = scratch.path().join("holder.sock");
        let _serving = UnixListener::bind(&socket).unwrap();
        let (service, holder) = greeted_pair();
        let (sender, outputs) = unbounded();
        let mut handle =
            HolderHandle::supervise(service, &socket, Box::new(ChannelSink(sender)), WAIT).unwrap();
        assert_eq!(handle.screen_source(), ScreenSource::Served(socket.clone()));
        assert_eq!(
            handle.take_initial_screen(),
            Some(SCREEN.to_vec()),
            "the screen is handed back, not pushed to the sink"
        );
        assert_eq!(handle.take_initial_screen(), None, "taken once");

        holder.shutdown(Shutdown::Both).unwrap();

        assert_eq!(
            outputs.recv_timeout(QUIET),
            Err(RecvTimeoutError::Timeout),
            "the reader ends without an exit or an unreadable report"
        );
        drop(handle);
        assert_eq!(
            outputs.recv_timeout(WAIT),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    /// A holder that stops reading fails the write after the write timeout
    /// rather than stalling the caller, and the broken connection is reported
    /// as unreadable output so the supervisor reconnects; later writes fail at
    /// once.
    #[test]
    fn a_holder_that_stops_reading_fails_the_write_and_is_reported() {
        let (service, _holder_never_reads) = greeted_pair();
        let (sender, outputs) = unbounded();
        let mut handle = HolderHandle::supervise(
            service,
            Path::new(UNSERVED_SOCKET),
            Box::new(ChannelSink(sender)),
            WAIT,
        )
        .unwrap();
        let flood = vec![b'x'; FLOOD_BYTES];

        let started = Instant::now();
        let failed = loop {
            assert!(started.elapsed() < WAIT, "a write never timed out");
            if let Err(error) = handle.write_input(&flood) {
                break error;
            }
        };

        assert!(failed.to_string().contains(WRITE_TIMED_OUT), "{failed}");
        assert!(matches!(
            outputs.recv_timeout(WAIT),
            Ok(ProcessOutput::Unreadable(reason)) if reason == WRITE_TIMED_OUT
        ));
        let quick = Instant::now();
        assert!(
            handle.kill().is_err(),
            "the broken connection writes nothing"
        );
        assert!(quick.elapsed() < HOLDER_WRITE_TIMEOUT);
    }
}
