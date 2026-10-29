use std::{
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
    client_message::ClientMessage,
    client_role::ClientRole,
    error::HolderError,
    holder_control::HolderControl,
    log_reader::read_log_range,
    reader::{Reader, spawn_reader},
    runner::abandon,
    shared_writer::SharedWriter,
};
use crate::domain::{
    port::{OutputSink, ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// Longest one write to the holder may wait for it to read, so a holder that
/// stops reading fails the request instead of stalling the service.
pub(super) const HOLDER_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Most bytes of logged output a handle reads back as it connects: what a
/// service's output tap keeps, so older output would only be dropped again.
const BACKLOG_MAX_BYTES: u64 = 256 * 1024;

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
    /// Where the log ended as the holder greeted; streamed output starts here.
    log_end: OutputCursor,
    /// Output logged before `log_end`, and where it starts, until taken.
    backlog: Option<(OutputCursor, Vec<u8>)>,
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
            log_end: greeting.log_end(),
            backlog: None,
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

    /// Reads what the child wrote to `log` before this handle connected, from
    /// `since` or the run's start and at most the last [`BACKLOG_MAX_BYTES`],
    /// for [`ProcessHandle::take_backlog`]. A log that cannot be read leaves
    /// none, and the output from here on is still streamed.
    pub(super) fn with_backlog(mut self, log: &Path, since: Option<OutputCursor>) -> Self {
        let from = since
            .unwrap_or(self.run_start)
            .max(self.run_start)
            .max(OutputCursor::new(
                self.log_end.offset().saturating_sub(BACKLOG_MAX_BYTES),
            ));
        if from < self.log_end {
            self.backlog = read_log_range(log, from, self.log_end)
                .ok()
                .filter(|(_, bytes)| !bytes.is_empty());
        }
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

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        self.backlog.take()
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

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        path::Path,
        time::{Duration, Instant},
    };

    use crossbeam_channel::{RecvTimeoutError, unbounded};

    use super::*;
    use crate::{
        adapter::{
            holder::{
                channel_sink::ChannelSink, protocol::HolderMessage, reader::HOLDER_LOST,
                shared_writer::WRITE_TIMED_OUT,
            },
            test_scratch::short_scratch,
        },
        domain::pty::ProcessOutput,
    };

    /// Bound on every wait in these tests; reaching it fails the test.
    const WAIT: Duration = Duration::from_secs(10);
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
            log_end: OutputCursor::new(0),
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
        let scratch = short_scratch(SCRATCH_PREFIX);
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
        let scratch = short_scratch(SCRATCH_PREFIX);
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
