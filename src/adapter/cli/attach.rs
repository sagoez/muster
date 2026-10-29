//! `muster attach`: takes over the terminal to show a running process's
//! screen and pass keystrokes to it until the detach key, like `tmux attach`.

use std::{
    fs::File,
    io::{self, Write},
    net::Shutdown,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::JoinHandle,
    time::Duration,
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::{Errno, read},
};

use super::{
    attach_end::AttachEnd, attach_event::AttachEvent, attach_pump::AttachPump,
    detach_scanner::DetachScanner, error::CliError, host_signals::HostSignals,
    host_terminal::HostTerminal, raw_mode_guard::RawModeGuard, session_alarm::SessionAlarm,
    session_thread::spawn_session_thread, signal_source::SignalSource,
};
use crate::{
    adapter::{
        client::{ClientError, ServiceClient, process::unexpected},
        holder::{
            ClientRole, HolderError, HolderMessage,
            client::{Greeting, greet},
        },
        sanitize::printable,
        service::{Located, ServiceRequest, ServiceResponse},
    },
    domain::{
        project::Project,
        value::{DetachKey, ProcessName},
    },
};

/// Longest the holder may take to send its screen.
const GREETING_TIMEOUT: Duration = Duration::from_secs(5);
/// Events the session's channel holds before its senders wait, so a terminal
/// slower than the process holds the holder back rather than memory growing.
const EVENT_CAPACITY: usize = 64;
/// How often the input reader checks whether the session ended.
const INPUT_POLL: Duration = Duration::from_millis(50);
/// Bytes read from the terminal at a time.
const INPUT_CHUNK: usize = 4096;
/// Opens the hint line.
const HINT_ATTACHED: &str = "attached to";
/// Marks a read-only attach in the hint line.
const HINT_READ_ONLY: &str = " (read-only)";
/// Closes the hint, after the detach key.
const HINT_DETACH: &str = "to detach";
/// Opens the window title shown while attached.
const TITLE_PREFIX: &str = "muster:";
/// Said after the session when the holder dropped typed input.
const DROPPED_INPUT: &str =
    "some of what was typed did not reach it: it was not reading its input fast enough";

/// Says the line a session opens with, before it takes the terminal over.
pub type Announce = Box<dyn FnOnce(&str) -> io::Result<()> + Send>;

/// Attaches `terminal` to the process of `project` named exactly `name`. The
/// holder counts the session for the service, so nothing is noted here.
/// `announce` says which process it attached to and how to detach, once the
/// process is reached and before the terminal is taken over.
///
/// # Errors
/// Returns [`ClientError::NotRunning`] when the process does not run, or the
/// session's failure.
pub fn attach_named(
    client: ServiceClient,
    project: &Project,
    name: ProcessName,
    terminal: HostTerminal,
    read_only: bool,
    detach: DetachKey,
    announce: Announce,
) -> Result<AttachEnd, CliError> {
    let target = match client.send(&ServiceRequest::Locate {
        project: project.root().clone(),
        name: name.clone(),
    })? {
        ServiceResponse::Located(located) => located,
        other => return Err(unexpected(other, project, Some(&name)).into()),
    };
    AttachSession::new(name, terminal, Box::new(HostSignals::default()), announce)
        .run(target, read_only, detach)
}

/// The hint naming the process and the detach key, shown before the
/// session takes over the terminal and in its window title while attached.
pub fn attach_hint(name: &ProcessName, detach: &DetachKey) -> String {
    printable(&format!("{name} - {detach} {HINT_DETACH}"))
}

/// A person's terminal attached to one process: its screen drawn, its
/// keystrokes passed on, until the detach key, the process's exit, a signal
/// or a broken connection ends it. The terminal is left as it was found on
/// every one of those ends.
pub struct AttachSession {
    /// The process, as the hint and errors name it.
    name: ProcessName,
    /// The terminal taken over.
    terminal: HostTerminal,
    /// Tells the session the terminal resized or a signal ends it.
    signals: Box<dyn SignalSource>,
    /// Says the opening line before the terminal is taken over.
    announce: Announce,
}

impl AttachSession {
    /// A session for the process `name` on `terminal`, hearing resizes and
    /// ending signals from `signals`, and saying its opening line through
    /// `announce`.
    pub fn new(
        name: ProcessName,
        terminal: HostTerminal,
        signals: Box<dyn SignalSource>,
        announce: Announce,
    ) -> Self {
        Self {
            name,
            terminal,
            signals,
            announce,
        }
    }

    /// Attaches to the holder at `target` and runs until the session ends.
    /// A `read_only` session passes no keystrokes or resizes on; `detach`
    /// ends it either way.
    ///
    /// # Errors
    /// Returns [`ClientError::NotRunning`] when the holder is gone,
    /// [`CliError::AttachLost`] when the connection breaks, or
    /// [`CliError::Terminal`] when the terminal cannot be driven.
    pub fn run(
        self,
        target: Located,
        read_only: bool,
        detach: DetachKey,
    ) -> Result<AttachEnd, CliError> {
        let Self {
            name,
            terminal,
            mut signals,
            announce,
        } = self;
        let (stream, greeting) = connect(&name, target.socket(), read_only)?;
        let (events, inbox) = sync_channel(EVENT_CAPACITY);
        let alarm = Arc::new(SessionAlarm::default());
        signals.listen(events.clone(), Arc::clone(&alarm))?;
        let closer = stream.try_clone().map_err(CliError::Terminal)?;
        let reader = spawn_holder_reader(
            stream.try_clone().map_err(CliError::Terminal)?,
            events.clone(),
            Arc::clone(&alarm),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let input = spawn_input_reader(
            terminal.input().try_clone().map_err(CliError::Terminal)?,
            Arc::clone(&stop),
            events,
            Arc::clone(&alarm),
        );
        let hint = attach_hint(&name, &detach);
        let note = if read_only { HINT_READ_ONLY } else { "" };
        let end = announce(&format!(
            "{HINT_ATTACHED} {name}{note} - {detach} {HINT_DETACH}"
        ))
        .and_then(|()| RawModeGuard::enter(&terminal))
        .map_err(CliError::Terminal)
        .and_then(|mut screen| {
            screen
                .set_title(&format!("{TITLE_PREFIX} {hint}"))
                .map_err(CliError::Terminal)?;
            let mut pump = AttachPump::new(
                &name,
                &terminal,
                stream,
                screen,
                DetachScanner::new(&detach),
                read_only,
                Arc::clone(&alarm),
            );
            let end = pump
                .first_screen(&greeting, &inbox)
                .and_then(|()| pump.run(&inbox));
            stop.store(true, Ordering::Relaxed);
            let dropped = pump.dropped_input();
            drop(pump);
            if dropped {
                let _ = draw_line(&terminal, &format!("{name}: {DROPPED_INPUT}"));
            }
            end
        });
        stop.store(true, Ordering::Relaxed);
        let _ = closer.shutdown(Shutdown::Both);
        signals.stop();
        drop(inbox);
        let _ = reader.join();
        let _ = input.join();
        if let Some(message) = alarm.take_panic_message() {
            let _ = draw_line(&terminal, &message);
        }
        end.map(|end| alarm.ending().map_or(end, AttachEnd::Signalled))
    }
}

/// Connects to the holder at `socket` and reads its screen.
///
/// # Errors
/// Returns [`ClientError::NotRunning`] when no holder serves the socket, or
/// [`CliError::AttachLost`] when the exchange fails otherwise.
fn connect(
    name: &ProcessName,
    socket: &Path,
    read_only: bool,
) -> Result<(UnixStream, Greeting), CliError> {
    let failed = |source: HolderError| match &source {
        HolderError::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            ClientError::NotRunning(name.clone()).into()
        },
        HolderError::NoSnapshot(_) => ClientError::NotRunning(name.clone()).into(),
        _ => CliError::AttachLost {
            name: name.clone(),
            source,
        },
    };
    let mut stream = UnixStream::connect(socket).map_err(|error| failed(error.into()))?;
    let greeting = greet(
        &mut stream,
        socket,
        ClientRole::Attach { read_only },
        GREETING_TIMEOUT,
    )
    .map_err(failed)?;
    stream
        .set_read_timeout(None)
        .map_err(|error| failed(error.into()))?;
    Ok((stream, greeting))
}

/// Prints `line` on the terminal while it is not in raw mode.
///
/// # Errors
/// Returns the I/O error when the terminal cannot be written.
fn draw_line(terminal: &HostTerminal, line: &str) -> io::Result<()> {
    let mut output = File::from(terminal.output().try_clone()?);
    writeln!(output, "{line}")
}

/// Reads the holder's messages into `events` until the connection ends.
fn spawn_holder_reader(
    mut stream: UnixStream,
    events: SyncSender<AttachEvent>,
    alarm: Arc<SessionAlarm>,
) -> JoinHandle<()> {
    spawn_session_thread(events, alarm, move |events| {
        loop {
            match HolderMessage::read_from(&mut stream) {
                Ok(message) => {
                    if events.send(AttachEvent::Holder(message)).is_err() {
                        return;
                    }
                },
                Err(error) => {
                    let _ = events.send(AttachEvent::HolderLost(error));
                    return;
                },
            }
        }
    })
}

/// Reads what is typed into `events` until the input ends or `stop` is set,
/// which it checks between reads so nothing typed after the session is
/// taken from the shell.
fn spawn_input_reader(
    input: OwnedFd,
    stop: Arc<AtomicBool>,
    events: SyncSender<AttachEvent>,
    alarm: Arc<SessionAlarm>,
) -> JoinHandle<()> {
    spawn_session_thread(events, alarm, move |events| {
        let timeout = Timespec::try_from(INPUT_POLL).ok();
        let mut chunk = [0; INPUT_CHUNK];
        loop {
            let mut ready = [PollFd::new(&input, PollFlags::IN)];
            let polled = poll(&mut ready, timeout.as_ref());
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let event = match polled {
                Ok(0) | Err(Errno::INTR) => continue,
                Ok(_) => match read(&input, &mut chunk) {
                    Ok(0) => AttachEvent::InputClosed,
                    Ok(length) => AttachEvent::Typed(chunk[..length].to_vec()),
                    Err(Errno::INTR | Errno::AGAIN) => continue,
                    Err(_) => AttachEvent::InputClosed,
                },
                Err(_) => AttachEvent::InputClosed,
            };
            let closed = matches!(event, AttachEvent::InputClosed);
            if events.send(event).is_err() || closed {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::pipe,
        os::unix::net::UnixListener,
        path::PathBuf,
        sync::{Mutex, PoisonError, mpsc},
        thread,
        time::Instant,
    };

    use bytesize::ByteSize as RawByteSize;
    use rustix::termios::Termios;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::{
                attach_pump::RESYNC_WAIT,
                ending_signal::EndingSignal,
                held_terminal::{KITTY_KEYBOARD_RESET, POP_TITLE, PUSH_TITLE},
                test_pty::{TestPty, WAIT, contains, same_settings, wait_until},
            },
            holder::{
                ClientMessage, HolderConfig, HolderControl, HolderId, HolderRecord, HolderServer,
                RejectReason,
            },
            terminal::emulator::RESET_TERMINAL,
        },
        domain::{
            color_scheme::ColorScheme,
            process::{StopPolicy, StopSignal},
            pty::{ExitCause, ExitOutcome, PtySize, SpawnRequest},
            value::{ByteSize, Cols, CommandLine, PositiveDuration, Rows},
        },
    };

    /// Short root for test sockets: macOS's long temp dir breaches `sun_path`.
    const SHORT_TEMP_ROOT: &str = "/tmp";
    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "ma-";
    /// Rows of the test terminal.
    const ROWS: u16 = 24;
    /// Columns of the test terminal.
    const COLS: u16 = 80;
    /// Scrollback of the test holder.
    const SCROLLBACK: usize = 100;
    /// First-supervisor deadline of a test holder, past any test's length.
    const SUPERVISOR_DEADLINE: Duration = Duration::from_secs(60);
    /// The detach key every test uses.
    const DETACH: &str = "ctrl-\\";
    /// The detach key's legacy byte.
    const DETACH_BYTE: &[u8] = b"\x1c";
    /// Pid a fake holder reports.
    const FAKE_PID: u32 = 4242;
    /// How long a test lets a refused keystroke travel before proving it did
    /// not arrive.
    const SETTLE: Duration = Duration::from_millis(200);
    /// How often a streaming fake holder writes output.
    const STREAM_INTERVAL: Duration = Duration::from_millis(5);
    /// The most a held Escape may take to reach the process while output
    /// streams: the escape wait plus scheduling slack.
    const ESCAPE_BOUND: Duration = Duration::from_millis(500);
    /// The most a resize may take to reach the process while output
    /// streams: the resize wait plus scheduling slack.
    const RESIZE_BOUND: Duration = Duration::from_millis(500);
    /// What the title shows while attached to `api`.
    const TITLE: &[u8] = b"\x1b]2;muster: api - ctrl-\\ to detach\x07";
    /// A helper's panic message the session holds until it is over.
    const HELD_PANIC: &str = "thread 'reader' panicked at a bug";
    /// Turns line wrapping off.
    const WRAP_OFF: &[u8] = b"\x1b[?7l";
    /// Turns line wrapping on.
    const WRAP_ON: &[u8] = b"\x1b[?7h";

    /// The process every test attaches to.
    fn name() -> ProcessName {
        ProcessName::try_new("api").unwrap()
    }

    /// The detach key every test uses.
    fn detach() -> DetachKey {
        DetachKey::try_new(DETACH).unwrap()
    }

    /// A terminal size of `rows` by `cols`.
    fn size(rows: u16, cols: u16) -> PtySize {
        PtySize::builder()
            .rows(Rows::new(rows))
            .cols(Cols::new(cols))
            .build()
    }

    /// A scratch directory under a short root, removed even on a panic.
    fn scratch() -> TempDir {
        tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap()
    }

    /// A repaint as a holder sends it: a full reset, then `text`.
    fn repaint(text: &str) -> Vec<u8> {
        [RESET_TERMINAL.as_bytes(), text.as_bytes()].concat()
    }

    /// Where in `drawn` `needle` first appears.
    fn position(drawn: &[u8], needle: &[u8]) -> Option<usize> {
        drawn
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Where a test sends the events a signal would, and the session's
    /// alarm, while the session listens.
    type SignalSlot = Arc<Mutex<Option<(SyncSender<AttachEvent>, Arc<SessionAlarm>)>>>;

    /// A signal source a test drives: it posts `preset` as soon as the
    /// session listens, then whatever the test sends through `slot`, and
    /// reports `late` as it is stopped, as signals arriving during teardown
    /// would, the way the real listener does: recorded in the alarm, posted
    /// only when there is room.
    struct ScriptedSignals {
        /// The session's event sender and alarm, while it listens.
        slot: SignalSlot,
        /// Events posted the moment the session listens.
        preset: Vec<AttachEvent>,
        /// Ending signals arriving while the session stops listening.
        late: Vec<EndingSignal>,
    }

    impl SignalSource for ScriptedSignals {
        fn listen(
            &mut self,
            events: SyncSender<AttachEvent>,
            alarm: Arc<SessionAlarm>,
        ) -> Result<(), CliError> {
            for event in self.preset.drain(..) {
                events.send(event).unwrap();
            }
            *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((events, alarm));
            Ok(())
        }

        fn stop(&mut self) {
            let listening = self
                .slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some((events, alarm)) = listening {
                for signal in self.late.drain(..) {
                    alarm.record_ending(signal);
                    let _ = events.try_send(AttachEvent::Ended(signal));
                }
            }
        }
    }

    /// A session running on a thread, with a way to send it events a signal
    /// would.
    struct Running {
        /// The session's thread.
        session: JoinHandle<Result<AttachEnd, CliError>>,
        /// Where signal events go.
        signals: SignalSlot,
    }

    impl Running {
        /// Attaches to `socket` from `pty`.
        fn start(pty: &TestPty, socket: PathBuf, read_only: bool) -> Self {
            Self::on(pty.terminal(), socket, read_only, Vec::new(), Vec::new())
        }

        /// Attaches to `socket` from `terminal`, with `preset` signal events
        /// already waiting and `late` ending signals arriving as signals stop.
        fn on(
            terminal: HostTerminal,
            socket: PathBuf,
            read_only: bool,
            preset: Vec<AttachEvent>,
            late: Vec<EndingSignal>,
        ) -> Self {
            let signals: SignalSlot = Arc::new(Mutex::new(None));
            let source = ScriptedSignals {
                slot: Arc::clone(&signals),
                preset,
                late,
            };
            let output = terminal.output().try_clone().unwrap();
            let announce: Announce =
                Box::new(move |line: &str| writeln!(File::from(output), "{line}"));
            let session = AttachSession::new(name(), terminal, Box::new(source), announce);
            let target = Located::builder()
                .socket(socket)
                .log(PathBuf::from("unused.log"))
                .build();
            let session = thread::spawn(move || session.run(target, read_only, detach()));
            Self { session, signals }
        }

        /// The session's event sender and alarm, once it listens.
        fn listening(&self) -> (SyncSender<AttachEvent>, Arc<SessionAlarm>) {
            let mut listening = None;
            wait_until("the session to listen", || {
                listening = self
                    .signals
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                listening.is_some()
            });
            listening.unwrap()
        }

        /// Sends `event` as a signal would, once the session listens.
        fn signal(&self, event: AttachEvent) {
            self.listening().0.send(event).unwrap();
        }

        /// The session's alarm, once it listens.
        fn alarm(&self) -> Arc<SessionAlarm> {
            self.listening().1
        }

        /// Waits for the session to end, answering how.
        fn end(self) -> Result<AttachEnd, CliError> {
            wait_until("the session to end", || self.session.is_finished());
            self.session.join().unwrap()
        }
    }

    /// Asserts the terminal behind `pty` was given back: raw mode off and
    /// the restore drawn after everything else.
    fn assert_restored(pty: &TestPty, cooked: &Termios) {
        assert!(same_settings(&pty.settings(), cooked), "raw mode is off");
        pty.wait_for(POP_TITLE.as_bytes());
        let drawn = pty.drawn();
        let tail = &drawn[position(&drawn, PUSH_TITLE.as_bytes()).unwrap_or(0)..];
        assert!(
            contains(tail, KITTY_KEYBOARD_RESET.as_bytes()),
            "kitty keyboard reset"
        );
        assert!(contains(tail, b"\x1b[?25h"), "the cursor is shown");
        assert!(
            position(&drawn, POP_TITLE.as_bytes()) > position(&drawn, TITLE),
            "the hint title is popped"
        );
    }

    /// A holder serving a real child on a thread, with a supervisor connected
    /// so it collects the exit and finishes cleanly.
    struct Holder {
        /// The holder's config.
        config: HolderConfig,
        /// The holder's thread, until it is joined.
        server: Option<JoinHandle<Result<(), HolderError>>>,
        /// The test's supervisor connection.
        supervisor: UnixStream,
        /// The scratch directory, removed on drop.
        _scratch: TempDir,
    }

    impl Holder {
        /// Starts a holder running `command` at the test terminal's size.
        fn start(command: &str) -> Self {
            let scratch = scratch();
            let dir = scratch.path();
            let request = SpawnRequest::builder()
                .process(name())
                .command(CommandLine::try_new(command).unwrap())
                .project(Some(dir.to_path_buf()))
                .size(size(ROWS, COLS))
                .scrollback(SCROLLBACK)
                .log_max_size(ByteSize::try_new(RawByteSize::mib(1)).unwrap())
                .color_scheme(ColorScheme::Dark)
                .stop(
                    StopPolicy::builder()
                        .signal(StopSignal::Terminate)
                        .grace_period(PositiveDuration::try_new(Duration::from_secs(5)).unwrap())
                        .build(),
                )
                .build();
            let config = HolderConfig::builder()
                .id(HolderId::generate().unwrap())
                .request(request)
                .socket(dir.join("sock").join("h.sock"))
                .record_dir(dir.join("records"))
                .log(dir.join("logs").join("api.log"))
                .supervisor_deadline(SUPERVISOR_DEADLINE)
                .build();
            let server_config = config.clone();
            let server = thread::spawn(move || HolderServer::run(server_config));
            let mut supervisor = None;
            wait_until("the holder socket", || {
                supervisor = UnixStream::connect(config.socket()).ok();
                supervisor.is_some()
            });
            let mut supervisor = supervisor.unwrap();
            greet(
                &mut supervisor,
                config.socket(),
                ClientRole::Supervisor,
                WAIT,
            )
            .unwrap();
            let mut drain = supervisor.try_clone().unwrap();
            drain.set_read_timeout(None).unwrap();
            thread::spawn(move || while HolderMessage::read_from(&mut drain).is_ok() {});
            Self {
                config,
                server: Some(server),
                supervisor,
                _scratch: scratch,
            }
        }

        /// The holder's socket.
        fn socket(&self) -> PathBuf {
            self.config.socket().clone()
        }

        /// Writes `bytes` to the child as another, writable client would.
        fn type_as_another_client(&self, bytes: &[u8]) {
            let mut client = UnixStream::connect(self.config.socket()).unwrap();
            greet(
                &mut client,
                self.config.socket(),
                ClientRole::Attach { read_only: false },
                WAIT,
            )
            .unwrap();
            ClientMessage::Input(bytes.to_vec())
                .write_to(&mut client)
                .unwrap();
        }

        /// Kills the child when it still runs, collects its exit and joins
        /// the holder.
        fn finish(mut self) {
            ClientMessage::Control(HolderControl::Kill)
                .write_to(&mut self.supervisor)
                .unwrap();
            let server = self.server.take().unwrap();
            wait_until("the holder to finish", || {
                let _ = ClientMessage::CollectExit.write_to(&mut self.supervisor);
                thread::sleep(Duration::from_millis(20));
                server.is_finished()
            });
            server.join().unwrap().unwrap();
        }
    }

    impl Drop for Holder {
        /// Kills the child of a holder a failed test left running.
        fn drop(&mut self) {
            if self.server.is_some()
                && let Some(record) = HolderRecord::load_all(self.config.record_dir()).pop()
            {
                // SAFETY: `kill` takes plain integers and touches no memory; a
                // stale pid at worst signals nothing.
                unsafe {
                    libc::kill(-(record.child_pid() as libc::pid_t), libc::SIGKILL);
                }
            }
        }
    }

    /// A scripted holder: it answers the hello with a repaint, then records
    /// what the client sends and writes what a test tells it to.
    struct FakeHolder {
        /// Its socket.
        socket: PathBuf,
        /// The client connection, once the client said hello.
        peer: Arc<Mutex<Option<UnixStream>>>,
        /// What the client sent, in order.
        received: mpsc::Receiver<ClientMessage>,
        /// The scratch directory, removed on drop.
        _scratch: TempDir,
    }

    impl FakeHolder {
        /// A holder greeting with `repaint` at the test terminal's size.
        fn start(repaint: &[u8]) -> Self {
            Self::sized(repaint, size(ROWS, COLS))
        }

        /// A holder greeting with `repaint` rendered at `screen`.
        fn sized(repaint: &[u8], screen: PtySize) -> Self {
            let scratch = scratch();
            let socket = scratch.path().join("f.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let peer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
            let (sender, received) = mpsc::channel();
            let shared = Arc::clone(&peer);
            let repaint = repaint.to_vec();
            thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let hello = ClientMessage::read_from(&mut stream).unwrap();
                let _ = sender.send(hello);
                HolderMessage::Snapshot {
                    repaint,
                    size: screen,
                    pid: FAKE_PID,
                    attached: 0,
                }
                .write_to(&mut stream)
                .unwrap();
                *shared.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(stream.try_clone().unwrap());
                while let Ok(message) = ClientMessage::read_from(&mut stream) {
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            });
            Self {
                socket,
                peer,
                received,
                _scratch: scratch,
            }
        }

        /// The client connection, once it said hello.
        fn peer(&self) -> UnixStream {
            let mut peer = None;
            wait_until("the client to say hello", || {
                peer = self
                    .peer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .map(|stream| stream.try_clone().unwrap());
                peer.is_some()
            });
            peer.unwrap()
        }

        /// Writes `message` to the client.
        fn send(&self, message: HolderMessage) {
            message.write_to(&mut self.peer()).unwrap();
        }

        /// Breaks the connection.
        fn close(&self) {
            self.peer().shutdown(Shutdown::Both).unwrap();
        }

        /// The next message the client sent.
        fn next(&self) -> ClientMessage {
            self.received.recv_timeout(WAIT).unwrap()
        }

        /// Everything else the client sent, once it closed the connection.
        fn rest(&self) -> Vec<ClientMessage> {
            self.received.iter().collect()
        }
    }

    /// The process exiting ends the session with its outcome, and the
    /// terminal is given back after the process's last output.
    #[test]
    fn session_ends_with_exited_and_restores_on_process_exit() {
        let holder = Holder::start("printf 'ready\\n'; read -r line; exit 3");
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket(), false);

        pty.wait_for(b"ready");
        pty.type_keys(b"go\r");
        let end = running.end();

        assert_eq!(
            end.unwrap(),
            AttachEnd::Exited(ExitOutcome::Failed(ExitCause::Status(3)))
        );
        assert_restored(&pty, &cooked);
        assert!(contains(
            &pty.drawn(),
            b"attached to api - ctrl-\\ to detach\r\n"
        ));
        holder.finish();
    }

    /// Ctrl-C reaches the process as the byte it is, and the session goes
    /// on until the detach key.
    #[test]
    fn ctrl_c_is_forwarded_and_session_continues() {
        let holder = Holder::start(
            "stty -isig; printf 'ready\\n'; while read -r line; do printf '%s' \"$line\" | od -An -tx1; done",
        );
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket(), false);

        pty.wait_for(b"ready");
        pty.type_keys(b"\x03a\r");
        pty.wait_for(b"03 61");
        assert!(
            !running.session.is_finished(),
            "Ctrl-C does not end the session"
        );
        pty.type_keys(b"b\r");
        pty.wait_for(b" 62");
        pty.type_keys(DETACH_BYTE);
        let end = running.end();

        assert_eq!(end.unwrap(), AttachEnd::Detached);
        assert_restored(&pty, &cooked);
        holder.finish();
    }

    /// A read-only session shows the process but passes none of its
    /// keystrokes on, and still detaches.
    #[test]
    fn read_only_session_sends_no_input() {
        let holder = Holder::start(
            "printf 'ready\\n'; read -r line; printf 'got:%s\\n' \"$line\"; sleep 600",
        );
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket(), true);

        pty.wait_for(b"ready");
        pty.type_keys(b"hello\r");
        thread::sleep(SETTLE);
        holder.type_as_another_client(b"world\n");
        pty.wait_for(b"got:world");
        pty.type_keys(DETACH_BYTE);
        let end = running.end();

        assert_eq!(end.unwrap(), AttachEnd::Detached);
        assert!(
            !contains(&pty.drawn(), b"hello"),
            "the keystrokes never reached the child"
        );
        assert!(contains(
            &pty.drawn(),
            b"attached to api (read-only) - ctrl-\\ to detach"
        ));
        assert_restored(&pty, &cooked);
        holder.finish();
    }

    /// The detach key ends the session and gives the terminal back; the
    /// holder saw the hello and the terminal's size, and
    /// none of the detach key. The hint is the window title while attached.
    #[test]
    fn a_detach_restores_the_terminal() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket.clone(), false);

        assert_eq!(holder.next(), ClientMessage::Hello {
            role: ClientRole::Attach { read_only: false }
        });
        assert_eq!(holder.next(), ClientMessage::Resize(size(ROWS, COLS)));
        pty.wait_for(b"screen");
        let drawn = pty.drawn();
        assert!(position(&drawn, PUSH_TITLE.as_bytes()) < position(&drawn, TITLE));
        pty.type_keys(b"ls\x1c");
        let end = running.end();

        assert_eq!(end.unwrap(), AttachEnd::Detached);
        assert_restored(&pty, &cooked);
        assert_eq!(holder.rest(), vec![ClientMessage::Input(b"ls".to_vec())]);
    }

    /// A hangup or terminate ends the session with that signal and the
    /// terminal given back.
    #[test]
    fn an_ending_signal_restores_the_terminal() {
        for signal in [EndingSignal::Hangup, EndingSignal::Terminate] {
            let holder = FakeHolder::start(&repaint("screen"));
            let pty = TestPty::open(ROWS, COLS);
            let cooked = pty.settings();
            let running = Running::start(&pty, holder.socket.clone(), false);

            pty.wait_for(b"screen");
            running.signal(AttachEvent::Ended(signal));
            let end = running.end();

            assert_eq!(end.unwrap(), AttachEnd::Signalled(signal));
            assert_restored(&pty, &cooked);
        }
    }

    /// Input that closes ends the session by the hangup signal that follows
    /// it, and as a detach when none follows.
    #[test]
    fn a_hangup_ends_by_its_signal_rather_than_the_closed_input() {
        for hangs_up in [true, false] {
            let holder = FakeHolder::start(&repaint("screen"));
            let pty = TestPty::open(ROWS, COLS);
            let (input, typing) = pipe().unwrap();
            let terminal = HostTerminal::new(
                OwnedFd::from(input),
                pty.terminal().output().try_clone().unwrap(),
            );
            let running = Running::on(
                terminal,
                holder.socket.clone(),
                false,
                Vec::new(),
                Vec::new(),
            );

            pty.wait_for(b"screen");
            drop(typing);
            thread::sleep(Duration::from_millis(30));
            if hangs_up {
                running.signal(AttachEvent::Ended(EndingSignal::Hangup));
            }
            let end = running.end();

            let expected = if hangs_up {
                AttachEnd::Signalled(EndingSignal::Hangup)
            } else {
                AttachEnd::Detached
            };
            assert_eq!(end.unwrap(), expected);
        }
    }

    /// A connection that breaks ends the session with the error and the
    /// terminal given back.
    #[test]
    fn a_broken_connection_restores_the_terminal() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        holder.close();
        let end = running.end();

        assert!(
            matches!(end, Err(CliError::AttachLost { .. })),
            "got {end:?}"
        );
        assert_restored(&pty, &cooked);
    }

    /// The first screen follows a full Kitty keyboard reset and never sends
    /// the terminal a full reset; a screen sent again clears the terminal
    /// and repaints it.
    #[test]
    fn a_resync_clears_the_screen_and_repaints() {
        let holder = FakeHolder::start(&repaint("first"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"first");
        holder.send(HolderMessage::Output(b"live".to_vec()));
        holder.send(HolderMessage::Snapshot {
            repaint: repaint("second"),
            size: size(ROWS, COLS),
            pid: FAKE_PID,
            attached: 0,
        });
        pty.wait_for(b"\x1b[H\x1b[2Jsecond");
        pty.type_keys(DETACH_BYTE);
        let end = running.end();

        assert_eq!(end.unwrap(), AttachEnd::Detached);
        let drawn = pty.drawn();
        assert!(
            !contains(&drawn, RESET_TERMINAL.as_bytes()),
            "no full reset reaches the terminal"
        );
        assert!(position(&drawn, KITTY_KEYBOARD_RESET.as_bytes()) < position(&drawn, b"first"));
        assert!(position(&drawn, b"first") < position(&drawn, b"live"));
        assert!(position(&drawn, b"live") < position(&drawn, b"second"));
    }

    /// A process that switched to the line-drawing charset leaves the
    /// terminal in ASCII after a detach.
    #[test]
    fn a_detach_after_a_line_drawing_charset_leaves_ascii() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        holder.send(HolderMessage::Output(b"\x1b(0lqk".to_vec()));
        pty.wait_for(b"lqk");
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);

        pty.wait_for(POP_TITLE.as_bytes());
        let drawn = pty.drawn();
        let last = |needle: &[u8]| drawn.windows(needle.len()).rposition(|w| w == needle);
        assert!(
            last(b"\x1b(B\x0f") > last(b"\x1b(0"),
            "the terminal ends in ASCII"
        );
    }

    /// A terminal that changes size resizes the process to match.
    #[test]
    fn a_window_change_resizes_the_process() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);
        holder.next();
        assert_eq!(holder.next(), ClientMessage::Resize(size(ROWS, COLS)));

        pty.resize(40, 120);
        running.signal(AttachEvent::Resized);

        assert_eq!(holder.next(), ClientMessage::Resize(size(40, 120)));
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);
    }

    /// A burst of resizes is passed on as one.
    #[test]
    fn a_burst_of_resizes_is_passed_on_once() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let burst = (0..10).map(|_| AttachEvent::Resized).collect();
        let running = Running::on(
            pty.terminal(),
            holder.socket.clone(),
            false,
            burst,
            Vec::new(),
        );
        holder.next();
        assert_eq!(holder.next(), ClientMessage::Resize(size(ROWS, COLS)));

        pty.wait_for(b"screen");
        thread::sleep(SETTLE);
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);

        assert_eq!(holder.rest(), vec![ClientMessage::Resize(size(ROWS, COLS))]);
    }

    /// A writable session whose terminal differs in size from the process
    /// resizes it before drawing anything, and draws the screen the holder
    /// redraws at its size instead of the one rendered at the old size.
    #[test]
    fn a_mismatched_size_resizes_before_the_first_screen() {
        let holder = FakeHolder::sized(&repaint("old-screen"), size(30, 100));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);

        holder.next();
        assert_eq!(holder.next(), ClientMessage::Resize(size(ROWS, COLS)));
        assert!(
            !contains(&pty.drawn(), b"old-screen"),
            "nothing drawn before the resize"
        );
        holder.send(HolderMessage::Snapshot {
            repaint: repaint("new-screen"),
            size: size(ROWS, COLS),
            pid: FAKE_PID,
            attached: 0,
        });
        pty.wait_for(b"new-screen");
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);

        assert!(
            !contains(&pty.drawn(), b"old-screen"),
            "the old-size screen is never drawn"
        );
    }

    /// When the holder refuses the resize, or redraws nothing in time, the
    /// screen it has is drawn with line wrapping off, so nothing wraps into
    /// the terminal's history.
    #[test]
    fn a_refused_or_unanswered_resize_draws_the_old_screen_unwrapped() {
        for refuses in [true, false] {
            let holder = FakeHolder::sized(&repaint("old-screen"), size(30, 100));
            let pty = TestPty::open(ROWS, COLS);
            let running = Running::start(&pty, holder.socket.clone(), false);

            holder.next();
            assert_eq!(holder.next(), ClientMessage::Resize(size(ROWS, COLS)));
            let asked = Instant::now();
            if refuses {
                holder.send(HolderMessage::Rejected(RejectReason::ChildUnavailable));
            }
            pty.wait_for(&[WRAP_OFF, b"old-screen", WRAP_ON].concat());
            if !refuses {
                assert!(asked.elapsed() >= RESYNC_WAIT / 2, "waited for the redraw");
            }
            pty.type_keys(DETACH_BYTE);
            assert_eq!(running.end().unwrap(), AttachEnd::Detached);
        }
    }

    /// A read-only session draws a screen of another size with line
    /// wrapping off, and resizes nothing.
    #[test]
    fn a_read_only_mismatched_screen_is_drawn_unwrapped() {
        let holder = FakeHolder::sized(&repaint("old-screen"), size(30, 100));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), true);

        pty.wait_for(&[WRAP_OFF, b"old-screen", WRAP_ON].concat());
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);
        holder.next();
        assert!(holder.rest().is_empty());
    }

    /// A read-only session sends the holder its hello alone: no input, no
    /// size, not even when the terminal resizes.
    #[test]
    fn a_read_only_session_sends_only_its_hello() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), true);

        assert_eq!(holder.next(), ClientMessage::Hello {
            role: ClientRole::Attach { read_only: true }
        });
        pty.wait_for(b"screen");
        pty.resize(40, 120);
        running.signal(AttachEvent::Resized);
        pty.type_keys(b"typed\x1b");
        thread::sleep(SETTLE);
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);

        assert!(holder.rest().is_empty());
    }

    /// A lone Escape reaches the process once no sequence follows it.
    #[test]
    fn a_lone_escape_reaches_the_process() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);
        holder.next();
        holder.next();

        pty.type_keys(b"\x1b");

        assert_eq!(holder.next(), ClientMessage::Input(b"\x1b".to_vec()));
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);
    }

    /// A held Escape reaches the process within its wait even while the
    /// process writes output every few milliseconds.
    #[test]
    fn an_escape_is_not_held_back_by_streaming_output() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);
        holder.next();
        holder.next();
        let (streaming, streamer) = stream_output(&holder);
        pty.wait_for(b"tick");

        let typed = Instant::now();
        pty.type_keys(b"\x1b");
        let message = holder.next();
        let waited = typed.elapsed();
        streaming.store(false, Ordering::Relaxed);
        streamer.join().unwrap();

        assert_eq!(message, ClientMessage::Input(b"\x1b".to_vec()));
        assert!(waited < ESCAPE_BOUND, "held for {waited:?}");
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);
    }

    /// Input the holder dropped is told once the terminal is given back.
    #[test]
    fn dropped_input_is_told_after_the_session() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        holder.send(HolderMessage::Rejected(RejectReason::InputBacklogged));
        thread::sleep(SETTLE);
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);

        let note = format!("api: {DROPPED_INPUT}");
        pty.wait_for(note.as_bytes());
        let drawn = pty.drawn();
        assert!(position(&drawn, POP_TITLE.as_bytes()) < position(&drawn, note.as_bytes()));
    }

    /// Writes output to the client every few milliseconds until the returned
    /// flag is cleared.
    fn stream_output(holder: &FakeHolder) -> (Arc<AtomicBool>, JoinHandle<()>) {
        let streaming = Arc::new(AtomicBool::new(true));
        let mut stream = holder.peer();
        let still = Arc::clone(&streaming);
        let streamer = thread::spawn(move || {
            while still.load(Ordering::Relaxed)
                && HolderMessage::Output(b"tick".to_vec())
                    .write_to(&mut stream)
                    .is_ok()
            {
                thread::sleep(STREAM_INTERVAL);
            }
        });
        (streaming, streamer)
    }

    /// A resize reaches the process within its wait even while the process
    /// writes output every few milliseconds.
    #[test]
    fn a_resize_is_not_held_back_by_streaming_output() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::start(&pty, holder.socket.clone(), false);
        holder.next();
        holder.next();
        let (streaming, streamer) = stream_output(&holder);
        pty.wait_for(b"tick");

        pty.resize(40, 120);
        let resized = Instant::now();
        running.signal(AttachEvent::Resized);
        let message = holder.next();
        let waited = resized.elapsed();
        streaming.store(false, Ordering::Relaxed);
        streamer.join().unwrap();

        assert_eq!(message, ClientMessage::Resize(size(40, 120)));
        assert!(waited < RESIZE_BOUND, "held for {waited:?}");
        pty.type_keys(DETACH_BYTE);
        assert_eq!(running.end().unwrap(), AttachEnd::Detached);
    }

    /// A process that exits while the session waits out a closed input ends
    /// the session with its exit.
    #[test]
    fn an_exit_during_the_hangup_wait_ends_the_session_with_it() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let (input, typing) = pipe().unwrap();
        let output = pty.terminal().output().try_clone().unwrap();
        let running = Running::on(
            HostTerminal::new(OwnedFd::from(input), output),
            holder.socket.clone(),
            false,
            Vec::new(),
            Vec::new(),
        );

        pty.wait_for(b"screen");
        drop(typing);
        thread::sleep(Duration::from_millis(30));
        holder.send(HolderMessage::Exited(ExitOutcome::Succeeded));

        assert_eq!(
            running.end().unwrap(),
            AttachEnd::Exited(ExitOutcome::Succeeded)
        );
    }

    /// An ending signal that arrives while the session tears down, with the
    /// process still writing, is still what the client ends by.
    #[test]
    fn a_signal_during_teardown_still_ends_the_client() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let running = Running::on(
            pty.terminal(),
            holder.socket.clone(),
            false,
            Vec::new(),
            vec![EndingSignal::Terminate],
        );
        holder.next();
        holder.next();
        let (streaming, streamer) = stream_output(&holder);

        pty.wait_for(b"tick");
        pty.type_keys(DETACH_BYTE);
        let end = running.end();
        streaming.store(false, Ordering::Relaxed);
        streamer.join().unwrap();

        assert_eq!(end.unwrap(), AttachEnd::Signalled(EndingSignal::Terminate));
    }

    /// An ending signal recorded only in the alarm, as when the inbox was
    /// full, still ends the session by it.
    #[test]
    fn an_ending_signal_the_inbox_did_not_carry_ends_the_session() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        running.alarm().record_ending(EndingSignal::Hangup);
        let end = running.end();

        assert_eq!(end.unwrap(), AttachEnd::Signalled(EndingSignal::Hangup));
        assert_restored(&pty, &cooked);
    }

    /// A helper failure recorded only in the alarm ends the session, and the
    /// helper's held panic message is printed once the terminal is back.
    #[test]
    fn a_helper_failure_the_inbox_did_not_carry_ends_the_session() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        let alarm = running.alarm();
        alarm.hold_panic_message(HELD_PANIC.to_string());
        alarm.record_helper_failure();
        let end = running.end();

        assert!(
            matches!(end, Err(CliError::AttachLost { .. })),
            "got {end:?}"
        );
        assert_restored(&pty, &cooked);
        pty.wait_for(HELD_PANIC.as_bytes());
        let drawn = pty.drawn();
        assert!(
            position(&drawn, POP_TITLE.as_bytes()) < position(&drawn, HELD_PANIC.as_bytes()),
            "the message follows the restore"
        );
    }

    /// A helper thread that failed ends the session as a broken connection,
    /// with the terminal given back.
    #[test]
    fn a_failed_helper_thread_ends_the_session() {
        let holder = FakeHolder::start(&repaint("screen"));
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, holder.socket.clone(), false);

        pty.wait_for(b"screen");
        running.signal(AttachEvent::HelperFailed);
        let end = running.end();

        assert!(
            matches!(end, Err(CliError::AttachLost { .. })),
            "got {end:?}"
        );
        assert_restored(&pty, &cooked);
    }

    /// A holder that is gone is reported as a process that is not running.
    #[test]
    fn a_missing_holder_is_not_running() {
        let scratch = scratch();
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let running = Running::start(&pty, scratch.path().join("gone.sock"), false);

        let end = running.end();

        assert!(
            matches!(end, Err(CliError::Client(ClientError::NotRunning(_)))),
            "got {end:?}"
        );
        assert!(same_settings(&pty.settings(), &cooked));
    }
}
