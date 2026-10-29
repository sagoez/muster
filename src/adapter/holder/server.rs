use std::{
    collections::HashMap,
    fs::{self, Permissions},
    net::Shutdown,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded};

use super::{
    byte_queue::ByteQueue,
    child_guard::ChildGuard,
    client_id::ClientId,
    client_message::ClientMessage,
    client_role::ClientRole,
    config::HolderConfig,
    connected_client::Client,
    error::HolderError,
    flow::Flow,
    frame::{Frame, write_frame},
    holder_control::HolderControl,
    holder_event::HolderEvent,
    holder_sink::HolderSink,
    log::RotatingLog,
    protocol::HolderMessage,
    record::HolderRecord,
    record_guard::RecordGuard,
    reject_reason::RejectReason,
    socket_guard::SocketGuard,
};
use crate::{
    adapter::{
        ipc,
        private_dir::{GROUP_OTHER_BITS, create_private_dir_all},
        process_identity::LocalProcessIdentity,
        pty::PortablePtyRunner,
        terminal::{KittyKeyboardTracker, TerminalEmulator},
    },
    domain::{
        port::{ProcessHandle, ProcessInput, ProcessRunner},
        pty::{ExitOutcome, ProcessOutput, PtyError, PtySize, SpawnRequest},
        value::{OutputCursor, Pid, ProjectRoot},
    },
};

/// Events the holder loop drains: child output plus every client's traffic.
/// Bounded so a stalled loop pushes back on the PTY reader instead of growing.
const EVENT_QUEUE_CAPACITY: usize = 1024;
/// Bytes queued to one client before it counts as stalled and is resynced.
const CLIENT_OUTBOX_BYTES: usize = 1024 * 1024;
/// Bytes of input queued for a child that is not reading before more is
/// refused.
const CHILD_INPUT_BYTES: usize = 1024 * 1024;
/// Longest a finishing holder waits for clients to take their last frames.
const FLUSH_GRACE: Duration = Duration::from_secs(2);
/// Mode of the socket: owner read and write only.
const PRIVATE_SOCKET_MODE: u32 = 0o600;
/// What the supervisor is told when the screen had to be reset.
const SCREEN_RESET_WARNING: &str =
    "the terminal emulator failed on the process's output; its screen was reset";

/// Runs one child under a PTY and serves it to clients until its exit has been
/// collected. A single loop owns the emulator, keyboard tracker and log; the PTY
/// reader, the acceptor and one reader per client only feed it events.
pub struct HolderServer {
    /// The running child.
    child: ChildGuard,
    /// The child's process id.
    child_pid: Pid,
    /// Bytes awaiting the child's input writer thread.
    input: ByteQueue<Vec<u8>>,
    /// The child's live screen.
    emulator: TerminalEmulator,
    /// The child's negotiated keyboard protocol.
    keyboard: KittyKeyboardTracker,
    /// The child's output log.
    log: RotatingLog,
    /// Where the child's run began in the log, before it wrote anything.
    run_start: OutputCursor,
    /// Connected clients by identity.
    clients: HashMap<ClientId, Client>,
    /// Each writable client's last requested size, most recent last.
    resizes: Vec<(ClientId, PtySize)>,
    /// The PTY's current size.
    size: PtySize,
    /// The child's exit, held until a supervisor collects it.
    exit: Option<ExitOutcome>,
    /// Whether a supervisor has ever said hello.
    supervised: bool,
    /// The writable attach-client count supervisors were last told.
    reported_attached: u32,
    /// The loop's single inbox.
    events: Receiver<HolderEvent>,
    /// Feeds the loop's inbox from grace timers.
    timers: Sender<HolderEvent>,
    /// Cloned into every client writer; all clones dropped means all flushed.
    flushing: Sender<()>,
    /// Disconnects once every client writer has finished.
    flushed: Receiver<()>,
}

impl HolderServer {
    /// Opens the log, spawns the child, binds the socket, writes the record and
    /// serves until a supervisor collects the child's exit, then removes the
    /// record and socket. Binding after the log and spawn means their failures
    /// come before any socket exists. A record-save failure is the exception:
    /// it follows the bind, and the runner recovers it by abandoning the holder
    /// and re-reading its error file. Any failure after the spawn kills the
    /// child. A holder no supervisor reaches by its deadline kills its child
    /// and finishes the same way.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the request names no project or a size
    /// outside the PTY bounds, the socket directory is not private, the
    /// socket, log or record cannot be written, or the child cannot be
    /// spawned.
    pub fn run(config: HolderConfig) -> Result<(), HolderError> {
        let request = config.request();
        let project = request
            .project()
            .clone()
            .and_then(|root| ProjectRoot::try_new(root).ok())
            .ok_or(HolderError::MissingProject)?;
        let log = RotatingLog::open(config.log().clone(), request.log_max_size())?;
        let (sender, events) = bounded(EVENT_QUEUE_CAPACITY);
        let handle =
            PortablePtyRunner.spawn(request.clone(), Box::new(HolderSink(sender.clone())))?;
        let child_started_at = SystemTime::now();
        let server = Self::new(handle, log, (sender.clone(), events), request)?;
        let listener = bind_private(config.socket())?;
        let guard = SocketGuard {
            path: config.socket().clone(),
            closing: Arc::new(AtomicBool::new(false)),
        };
        let record = HolderRecord::builder()
            .id(config.id().clone())
            .project(project)
            .process(request.process().clone())
            .command(request.command().clone())
            .stop(request.stop())
            .launch_environment(
                request
                    .launch_environment()
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            )
            .socket(config.socket().clone())
            .holder_pid(own_pid()?)
            .child_pid(server.child_pid)
            .child_start_token(LocalProcessIdentity::start_token(server.child_pid))
            .log(config.log().clone())
            .child_started_at(child_started_at)
            .history(request.history().clone())
            .build();
        record.save(config.record_dir())?;
        let record = RecordGuard {
            record,
            dir: config.record_dir().clone(),
            removed: false,
        };
        spawn_timer(
            config.supervisor_deadline(),
            HolderEvent::SupervisorDeadline,
            sender.clone(),
        );
        spawn_acceptor(listener, sender, Arc::clone(&guard.closing));
        server.serve_then_close()?;
        record.remove()
    }

    /// Serves until the exit is collected, then flushes and closes every
    /// connection. On a serving error the server is dropped instead, which
    /// kills a child still running.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a message cannot be encoded.
    fn serve_then_close(mut self) -> Result<(), HolderError> {
        self.serve()?;
        self.close();
        Ok(())
    }

    /// Wraps a spawned child, starting the thread that writes its input, with
    /// a terminal sized and kept as `request` asks. Kills the child when it
    /// cannot be served.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the child reports no pid or input writer.
    fn new(
        handle: Box<dyn ProcessHandle>,
        log: RotatingLog,
        (timers, events): (Sender<HolderEvent>, Receiver<HolderEvent>),
        request: &SpawnRequest,
    ) -> Result<Self, HolderError> {
        let size = request.size();
        let child = ChildGuard {
            handle,
            exited: false,
        };
        let child_pid = child
            .handle
            .process_id()
            .ok_or(HolderError::MissingChildPid)?;
        let child_input = child.handle.input().ok_or(HolderError::MissingChildInput)?;
        let input = ByteQueue::new(CHILD_INPUT_BYTES);
        spawn_child_writer(child_input, input.consumer());
        let (flushing, flushed) = bounded(0);
        Ok(Self {
            child,
            child_pid,
            input,
            emulator: TerminalEmulator::new(
                size.rows(),
                size.cols(),
                request.scrollback(),
                request.color_scheme(),
            ),
            keyboard: KittyKeyboardTracker::default(),
            run_start: log.end(),
            log,
            clients: HashMap::new(),
            resizes: Vec::new(),
            size,
            exit: None,
            supervised: false,
            reported_attached: 0,
            events,
            timers,
            flushing,
            flushed,
        })
    }

    /// Lets every client writer send what is queued, bounded by
    /// [`FLUSH_GRACE`], then closes every connection.
    fn close(self) {
        let Self {
            clients,
            flushing,
            flushed,
            ..
        } = self;
        let streams = clients
            .into_values()
            .map(|client| client.stream)
            .collect::<Vec<_>>();
        drop(flushing);
        let _ = flushed.recv_timeout(FLUSH_GRACE);
        for stream in streams {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    /// Drains events until a supervisor collects the child's exit, or until
    /// the first-supervisor deadline passes with none having said hello, when
    /// the child is killed.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a message cannot be encoded.
    fn serve(&mut self) -> Result<(), HolderError> {
        while let Ok(event) = self.events.recv() {
            match event {
                HolderEvent::Child(ProcessOutput::Chunk(bytes)) => self.on_output(bytes)?,
                HolderEvent::Child(ProcessOutput::Exited(outcome)) => self.on_exit(outcome)?,
                // Unreachable by construction: the child's PTY reader only
                // reports chunks and the exit; only a holder connection's
                // reader reports the rest.
                HolderEvent::Child(
                    ProcessOutput::Unreadable(_)
                    | ProcessOutput::Warning(_)
                    | ProcessOutput::Attached(_)
                    | ProcessOutput::Placed { .. }
                    | ProcessOutput::Repaint { .. },
                ) => {},
                HolderEvent::GraceElapsed if !self.child.exited => {
                    let _ = self.child.handle.kill();
                },
                HolderEvent::GraceElapsed => {},
                HolderEvent::SupervisorDeadline if !self.supervised => {
                    if !self.child.exited {
                        let _ = self.child.handle.kill();
                    }
                    return Ok(());
                },
                HolderEvent::SupervisorDeadline => {},
                HolderEvent::Connected { client, stream } => self.on_connected(client, stream),
                HolderEvent::Message { client, message } => {
                    if self.on_message(client, message)? == Flow::Finish {
                        return Ok(());
                    }
                },
                HolderEvent::Disconnected { client } => self.drop_client(client),
            }
        }
        Ok(())
    }

    /// Holds the child's exit for collection and tells every client. The
    /// supervisor gets the final screen first, since the service keeps no
    /// screen of its own and shows that one once the holder is gone.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the screen or the exit cannot be
    /// encoded.
    fn on_exit(&mut self, outcome: ExitOutcome) -> Result<(), HolderError> {
        self.child.exited = true;
        self.exit = Some(outcome.clone());
        let supervisors: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.role == Some(ClientRole::Supervisor))
            .map(|(id, _)| *id)
            .collect();
        for client in supervisors {
            self.send(client, self.snapshot())?;
        }
        self.broadcast(HolderMessage::Exited(outcome))
    }

    /// Feeds child output to the emulator and keyboard tracker, answers any
    /// terminal queries they raise, logs it and broadcasts it. Output the
    /// emulator panics on leaves it blank instead of ending the holder, and
    /// with it the child: every client is resynced to that screen and warned.
    /// That chunk still reaches the keyboard tracker and its query replies are
    /// still sent, so the child neither loses a protocol change nor waits on
    /// an answer.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the output cannot be encoded.
    fn on_output(&mut self, bytes: Vec<u8>) -> Result<(), HolderError> {
        let reset = self.emulator.process_or_reset(&bytes);
        let mut replies = self.emulator.take_pty_writes();
        replies.extend(self.keyboard.observe(&bytes));
        if !replies.is_empty() {
            let length = replies.len();
            self.input.try_push(replies, length);
        }
        let at = self.log.end();
        // A full disk must not take a live process down with its log, and a
        // detached holder has nowhere to report the failure.
        let _ = self.log.append(&bytes);
        if reset {
            self.resync_all()?;
            return self.broadcast(HolderMessage::Warning(SCREEN_RESET_WARNING.to_string()));
        }
        self.broadcast(HolderMessage::Output { at, bytes })
    }

    /// Registers a new connection and starts its writer thread.
    fn on_connected(&mut self, client: ClientId, stream: UnixStream) {
        let Ok(writer) = stream.try_clone() else {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        };
        let outbox = ByteQueue::new(CLIENT_OUTBOX_BYTES);
        spawn_writer(writer, outbox.consumer(), self.flushing.clone());
        self.clients.insert(client, Client {
            role: None,
            stream,
            outbox,
        });
    }

    /// Applies one client message according to the client's role.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a reply cannot be encoded.
    fn on_message(
        &mut self,
        client: ClientId,
        message: ClientMessage,
    ) -> Result<Flow, HolderError> {
        let Some(role) = self.clients.get(&client).map(|entry| entry.role) else {
            return Ok(Flow::Continue);
        };
        let Some(role) = role else {
            return match message {
                ClientMessage::Hello { role } => {
                    self.on_hello(client, role).map(|()| Flow::Continue)
                },
                ClientMessage::Input(_)
                | ClientMessage::Resize(_)
                | ClientMessage::Control(_)
                | ClientMessage::CollectExit => self.reject(client, RejectReason::HelloRequired),
            };
        };
        match message {
            ClientMessage::Hello { .. } => self.reject(client, RejectReason::AlreadyIntroduced),
            ClientMessage::Input(_) | ClientMessage::Resize(_) if !role.writable() => {
                self.reject(client, RejectReason::ReadOnly)
            },
            ClientMessage::Control(_) | ClientMessage::CollectExit
                if role != ClientRole::Supervisor =>
            {
                self.reject(client, RejectReason::SupervisorOnly)
            },
            ClientMessage::Input(bytes) => {
                let length = bytes.len();
                if self.input.try_push(bytes, length) {
                    Ok(Flow::Continue)
                } else {
                    self.reject(client, RejectReason::InputBacklogged)
                }
            },
            ClientMessage::Resize(size) => {
                self.resizes.retain(|(id, _)| *id != client);
                self.resizes.push((client, size));
                match self.apply_size(size) {
                    Ok(true) => self.resync_attached().map(|()| Flow::Continue),
                    Ok(false) => Ok(Flow::Continue),
                    Err(error) => self.reject_on_failure(client, Err(error)),
                }
            },
            ClientMessage::Control(control) => {
                let controlled = self.control(control);
                self.reject_on_failure(client, controlled)
            },
            ClientMessage::CollectExit if self.exit.is_some() => Ok(Flow::Finish),
            ClientMessage::CollectExit => self.reject(client, RejectReason::NotExited),
        }
    }

    /// Fixes a client's role and sends it the live screen, then the held exit
    /// when the child has already exited. A new supervisor evicts the previous
    /// one, so exactly one connection speaks for the service.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the snapshot cannot be encoded.
    fn on_hello(&mut self, client: ClientId, role: ClientRole) -> Result<(), HolderError> {
        if role == ClientRole::Supervisor {
            self.supervised = true;
            let previous = self
                .clients
                .iter()
                .filter(|(_, entry)| entry.role == Some(ClientRole::Supervisor))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            for evicted in previous {
                self.drop_client(evicted);
            }
        }
        let Some(entry) = self.clients.get_mut(&client) else {
            return Ok(());
        };
        entry.role = Some(role);
        if role == ClientRole::Supervisor {
            self.reported_attached = self.writable_attached();
        }
        self.resync(client)?;
        self.report_attached()
    }

    /// Replaces whatever `client` has queued with the live screen, then the
    /// held exit when the child has already exited.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the snapshot cannot be encoded.
    fn resync(&mut self, client: ClientId) -> Result<(), HolderError> {
        let mut frames = vec![Frame::try_from(self.snapshot())?];
        if let Some(outcome) = &self.exit {
            frames.push(Frame::try_from(HolderMessage::Exited(outcome.clone()))?);
        }
        let Some(entry) = self.clients.get(&client) else {
            return Ok(());
        };
        entry.outbox.clear();
        let delivered = frames.into_iter().all(|frame| {
            let length = frame.wire_len();
            entry.outbox.force_push(frame, length)
        });
        if !delivered {
            self.drop_client(client);
        }
        Ok(())
    }

    /// The live screen and modes, as a snapshot a client rebuilds them from.
    fn snapshot(&self) -> HolderMessage {
        let mut repaint = self.emulator.repaint();
        repaint.extend(self.keyboard.restore_sequence());
        HolderMessage::Snapshot {
            repaint,
            size: self.size,
            pid: self.child_pid,
            attached: self.writable_attached(),
            run_start: self.run_start,
            log_end: self.log.end(),
        }
    }

    /// Runs a supervisor's control request against the child. A graceful stop
    /// also arms a timer that force-kills the child once its grace runs out,
    /// so the stop completes even when the supervisor has gone.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the child refuses the signal.
    fn control(&mut self, control: HolderControl) -> Result<(), PtyError> {
        let handle = &mut self.child.handle;
        match control {
            HolderControl::Pause => handle.pause(),
            HolderControl::Resume => handle.resume(),
            HolderControl::Terminate { signal, grace } => {
                handle.terminate(signal, grace)?;
                spawn_timer(grace, HolderEvent::GraceElapsed, self.timers.clone());
                Ok(())
            },
            HolderControl::Kill => handle.kill(),
        }
    }

    /// Resyncs every attach client, whose screen was laid out for a size the
    /// PTY no longer has, with the screen reflowed to the current one.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a snapshot cannot be encoded.
    fn resync_attached(&mut self) -> Result<(), HolderError> {
        self.resync_where(|role| matches!(role, ClientRole::Attach { .. }))
    }

    /// Resyncs every client that has said hello.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a snapshot cannot be encoded.
    fn resync_all(&mut self) -> Result<(), HolderError> {
        self.resync_where(|_| true)
    }

    /// Resyncs every client whose role satisfies `wanted`.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when a snapshot cannot be encoded.
    fn resync_where(&mut self, wanted: impl Fn(ClientRole) -> bool) -> Result<(), HolderError> {
        let chosen: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.role.is_some_and(&wanted))
            .map(|(id, _)| *id)
            .collect();
        for client in chosen {
            self.resync(client)?;
        }
        Ok(())
    }

    /// The writable attach clients connected.
    fn writable_attached(&self) -> u32 {
        let count = self
            .clients
            .values()
            .filter(|entry| matches!(entry.role, Some(ClientRole::Attach { read_only: false })))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Tells every supervisor the writable attach-client count when it
    /// changed since last told, so the service never counts presence itself.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the message cannot be encoded.
    fn report_attached(&mut self) -> Result<(), HolderError> {
        let attached = self.writable_attached();
        if attached == self.reported_attached {
            return Ok(());
        }
        self.reported_attached = attached;
        let supervisors: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.role == Some(ClientRole::Supervisor))
            .map(|(id, _)| *id)
            .collect();
        for client in supervisors {
            self.send(client, HolderMessage::Attached(attached))?;
        }
        Ok(())
    }

    /// Resizes the PTY when `size` differs from the current one, then the
    /// emulator, so a refused resize leaves both at the size the child has.
    /// Returns whether the size changed.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the PTY refuses the resize.
    fn apply_size(&mut self, size: PtySize) -> Result<bool, PtyError> {
        if size == self.size {
            return Ok(false);
        }
        self.child.handle.resize(size)?;
        self.size = size;
        self.emulator.set_size(size.rows(), size.cols());
        Ok(true)
    }

    /// Forgets a client and, when it set the current size, falls back to the
    /// most recent remaining writable client's size. A supervisor is told
    /// when the writable attach clients changed.
    fn drop_client(&mut self, client: ClientId) {
        if let Some(entry) = self.clients.remove(&client) {
            let _ = entry.stream.shutdown(Shutdown::Both);
        }
        let _ = self.report_attached();
        let was_latest = self.resizes.last().is_some_and(|(id, _)| *id == client);
        self.resizes.retain(|(id, _)| *id != client);
        if was_latest
            && let Some(&(_, size)) = self.resizes.last()
            && let Ok(true) = self.apply_size(size)
        {
            let _ = self.resync_attached();
        }
    }

    /// Answers `client` with a rejection when `result` failed.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the rejection cannot be encoded.
    fn reject_on_failure(
        &mut self,
        client: ClientId,
        result: Result<(), PtyError>,
    ) -> Result<Flow, HolderError> {
        match result {
            Ok(()) => Ok(Flow::Continue),
            Err(_) => self.reject(client, RejectReason::ChildUnavailable),
        }
    }

    /// Tells `client` its message was refused.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the rejection cannot be encoded.
    fn reject(&mut self, client: ClientId, reason: RejectReason) -> Result<Flow, HolderError> {
        self.send(client, HolderMessage::Rejected(reason))?;
        Ok(Flow::Continue)
    }

    /// Queues `message` for one client.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the message cannot be encoded.
    fn send(&mut self, client: ClientId, message: HolderMessage) -> Result<(), HolderError> {
        let frame = Frame::try_from(message)?;
        let length = frame.wire_len();
        match self.clients.get(&client) {
            Some(entry) if !entry.outbox.try_push(frame, length) => self.on_stalled(client),
            _ => Ok(()),
        }
    }

    /// Queues `message` for every client that has said hello.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the message cannot be encoded.
    fn broadcast(&mut self, message: HolderMessage) -> Result<(), HolderError> {
        let frame = Frame::try_from(message)?;
        let length = frame.wire_len();
        let stalled = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.role.is_some())
            .filter(|(_, entry)| !entry.outbox.try_push(frame.clone(), length))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for client in stalled {
            self.on_stalled(client)?;
        }
        Ok(())
    }

    /// Handles a client whose outbox is full: one that has said hello is
    /// resynced so it never stalls the loop, one that has not is dropped.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the resync snapshot cannot be encoded.
    fn on_stalled(&mut self, client: ClientId) -> Result<(), HolderError> {
        match self.clients.get(&client).and_then(|entry| entry.role) {
            Some(_) => self.resync(client),
            None => {
                self.drop_client(client);
                Ok(())
            },
        }
    }
}

/// This holder's own pid, as its record names it.
///
/// # Errors
/// Returns [`HolderError::UnrecordablePid`] for a pid no [`Pid`] can hold.
fn own_pid() -> Result<Pid, HolderError> {
    let raw = process::id();
    Pid::try_new(raw).map_err(|_| HolderError::UnrecordablePid(raw))
}

/// Binds the holder socket so only its owner can reach it: the directory is
/// created owner-only and refused when other users can enter it, and the
/// socket itself is owner read and write only.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the directory is shared or
/// absent from the path, or the I/O error from creating or binding it.
fn bind_private(path: &Path) -> Result<UnixListener, HolderError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| HolderError::InsecureSocketDir(path.to_path_buf()))?;
    ensure_private_dir(parent)?;
    let listener = ipc::bind(path)?;
    if let Err(error) = fs::set_permissions(path, Permissions::from_mode(PRIVATE_SOCKET_MODE)) {
        ipc::unlink(path);
        return Err(error.into());
    }
    Ok(listener)
}

/// Creates `dir` owner-only when absent, and refuses an existing one other
/// users can enter rather than changing a possibly shared directory's mode.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the directory is shared, or
/// the I/O error from creating or inspecting it.
pub(super) fn ensure_private_dir(dir: &Path) -> Result<(), HolderError> {
    create_private_dir_all(dir)?;
    if fs::metadata(dir)?.permissions().mode() & GROUP_OTHER_BITS != 0 {
        return Err(HolderError::InsecureSocketDir(dir.to_path_buf()));
    }
    Ok(())
}

/// Sends `event` to the holder loop once `after` has passed. A loop that has
/// finished simply never receives it.
fn spawn_timer(after: Duration, event: HolderEvent, timers: Sender<HolderEvent>) {
    thread::spawn(move || {
        thread::sleep(after);
        let _ = timers.send(event);
    });
}

/// Writes queued input to the child until the holder drops the queue. A child
/// that stops reading blocks only this thread, never the holder loop.
fn spawn_child_writer(
    mut input: Box<dyn ProcessInput>,
    queued: impl Iterator<Item = Vec<u8>> + Send + 'static,
) {
    thread::spawn(move || {
        for bytes in queued {
            let _ = input.write_input(&bytes);
        }
    });
}

/// Accepts connections until the holder closes, giving each a reader thread
/// that forwards its messages to the holder loop.
fn spawn_acceptor(listener: UnixListener, events: Sender<HolderEvent>, closing: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut next = 0;
        for stream in listener.incoming() {
            if closing.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else {
                continue;
            };
            let Ok(reader) = stream.try_clone() else {
                continue;
            };
            let client = ClientId(next);
            next += 1;
            if events
                .send(HolderEvent::Connected { client, stream })
                .is_err()
            {
                break;
            }
            spawn_reader(client, reader, events.clone());
        }
    });
}

/// Forwards one client's messages to the holder loop until the connection
/// closes or sends something unreadable, then reports the disconnect.
fn spawn_reader(client: ClientId, mut stream: UnixStream, events: Sender<HolderEvent>) {
    thread::spawn(move || {
        while let Ok(message) = ClientMessage::read_from(&mut stream) {
            if events
                .send(HolderEvent::Message { client, message })
                .is_err()
            {
                return;
            }
        }
        let _ = events.send(HolderEvent::Disconnected { client });
    });
}

/// Writes queued frames to one client until its outbox is dropped and drained
/// or a write fails, shutting the connection down so its reader ends too.
/// `flushing` is dropped on return, telling a finishing holder it is done.
fn spawn_writer(
    mut stream: UnixStream,
    frames: impl Iterator<Item = Frame> + Send + 'static,
    flushing: Sender<()>,
) {
    thread::spawn(move || {
        for frame in frames {
            if write_frame(&mut stream, &frame).is_err() {
                break;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        drop(flushing);
    });
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::{fs::PermissionsExt, net::UnixStream},
        path::{Path, PathBuf},
        sync::Mutex,
        thread,
        time::{Duration, SystemTime},
    };

    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            holder::{
                ClientMessage, ClientRole, FrameTag, HolderControl, HolderId, HolderMessage,
                HolderRecord, RejectReason,
                client::repaint_text,
                fake_child::FakeChild,
                harness::Harness,
                server_testing::{
                    COLS, FAKE_PID, ROWS, SCROLLBACK, SOCKET_DIR, SOCKET_FILE, SUPERVISOR_DEADLINE,
                    WAIT, request, scratch, size, wait_until,
                },
                testing::{test_request, test_stop},
            },
            signal_target::SignalTarget,
            terminal::TerminalEmulator,
        },
        domain::{
            color_scheme::ColorScheme,
            pty::ExitOutcome,
            value::{ByteSize, CommandLine, OutputCursor, ProcessName, Rows},
        },
    };

    /// A first-supervisor deadline short enough to test.
    const SHORT_SUPERVISOR_DEADLINE: Duration = Duration::from_millis(300);
    /// Output a flooding child writes: past what any client outbox and socket
    /// buffer can hold, so only a holder that never blocks on clients keeps up.
    const FLOOD_BYTES: usize = 6 * 1024 * 1024;
    /// Output a child writes just before it exits, far beyond a socket buffer.
    const TAIL_BYTES: usize = 512 * 1024;
    /// Input sent to a child that never reads, far beyond a PTY input buffer.
    const STUCK_INPUT_BYTES: usize = 4 * 1024 * 1024;
    /// The socket's expected mode: owner read and write only.
    const EXPECTED_SOCKET_MODE: u32 = 0o600;
    /// The socket directory's expected mode: owner only.
    const EXPECTED_DIR_MODE: u32 = 0o700;
    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    #[test]
    fn attach_receives_snapshot_with_prior_output_then_live_output() {
        let harness = Harness::start("printf before; read x; printf after");
        wait_until("prior output in the log", || {
            harness.log().contains("before")
        });

        let (mut client, snapshot) = harness.connect(ClientRole::Attach { read_only: false });
        let HolderMessage::Snapshot {
            repaint, size, pid, ..
        } = snapshot
        else {
            panic!("expected a snapshot, got {snapshot:?}");
        };
        let mut rebuilt =
            TerminalEmulator::new(size.rows(), size.cols(), SCROLLBACK, ColorScheme::Dark);
        rebuilt.process(&repaint);
        assert!(rebuilt.screen_text().contains("before"));
        assert_eq!(pid, harness.record().unwrap().child_pid());

        client.send(ClientMessage::Input(b"\n".to_vec()));
        client.output_until("after");
        harness.finish();
    }

    /// Each output frame names where it starts in the log, so the frames tile
    /// the log with no gap or overlap and end where the log ends.
    #[test]
    fn output_frames_carry_their_log_offsets() {
        let harness = Harness::start("read x; seq 1 2000; read y");
        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Input(b"\n".to_vec()));
        supervisor.output_until("2000\r\n");

        let frames = &supervisor.frames;
        assert_eq!(frames[0].0, OutputCursor::new(0));
        for pair in frames.windows(2) {
            assert_eq!(
                pair[1].0,
                pair[0].0.advanced(pair[0].1 as u64),
                "{frames:?}"
            );
        }
        let (last_at, last_len) = *frames.last().unwrap();
        let log_len = fs::metadata(harness.config.log()).unwrap().len();
        assert_eq!(
            last_at.advanced(last_len as u64),
            OutputCursor::new(log_len)
        );
        harness.finish();
    }

    #[test]
    fn two_attached_clients_both_receive_output() {
        let harness = Harness::start(r#"read x; printf "line:%s" "$x""#);
        let (mut first, _) = harness.connect(ClientRole::Attach { read_only: false });
        let (mut second, _) = harness.connect(ClientRole::Attach { read_only: false });

        first.send(ClientMessage::Input(b"hi\n".to_vec()));
        first.output_until("line:hi");
        second.output_until("line:hi");
        harness.finish();
    }

    #[test]
    fn read_only_client_input_is_rejected_and_not_written() {
        let harness = Harness::start(r#"read x; printf "got:%s" "$x""#);
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });
        watcher.send(ClientMessage::Input(b"secret\n".to_vec()));
        assert_eq!(
            watcher.recv_until(|message| matches!(message, HolderMessage::Rejected(_))),
            HolderMessage::Rejected(RejectReason::ReadOnly)
        );

        let (mut writer, _) = harness.connect(ClientRole::Attach { read_only: false });
        writer.send(ClientMessage::Input(b"ok\n".to_vec()));
        let seen = watcher.output_until("got:ok");
        assert!(!seen.contains("secret"));
        assert!(!harness.log().contains("secret"));
        harness.finish();
    }

    #[test]
    fn resize_follows_most_recent_writable_client() {
        let harness = Harness::start("trap 'stty size' WINCH; while :; do sleep 0.05; done");
        let (mut first, _) = harness.connect(ClientRole::Attach { read_only: false });
        let (mut second, _) = harness.connect(ClientRole::Attach { read_only: false });
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });

        first.send(ClientMessage::Resize(size(30, 100)));
        wait_until("the first client's size", || {
            harness.current_size() == size(30, 100)
        });
        second.send(ClientMessage::Resize(size(20, 60)));
        wait_until("the second client's size", || {
            harness.current_size() == size(20, 60)
        });
        first.output_until("20 60");

        watcher.send(ClientMessage::Resize(size(10, 10)));
        assert_eq!(
            watcher.recv_until(|message| matches!(message, HolderMessage::Rejected(_))),
            HolderMessage::Rejected(RejectReason::ReadOnly)
        );
        assert_eq!(harness.current_size(), size(20, 60));

        drop(second);
        wait_until("the size to fall back to the first client", || {
            harness.current_size() == size(30, 100)
        });
        harness.finish();
    }

    /// A one-column resize, as from a one-column terminal, is refused, so a
    /// wide character the child then prints never reaches a screen too narrow
    /// for it and the child keeps running. No `PtySize` can be one column, so
    /// the frame is written raw.
    #[test]
    fn a_one_column_resize_then_a_wide_character_keeps_the_child_alive() {
        let harness = Harness::start(r"read x; printf '\347\225\214 done'; read y");
        let (mut narrow, _) = harness.connect(ClientRole::Attach { read_only: false });
        let [rows_high, rows_low] = ROWS.to_be_bytes();
        let one_column = Frame::builder()
            .tag(FrameTag::Resize)
            .payload(vec![rows_high, rows_low, 0, 1])
            .build();
        write_frame(&mut narrow.stream, &one_column).unwrap();
        narrow.expect_closed();
        assert_eq!(harness.current_size(), size(ROWS, COLS));

        let (mut writer, _) = harness.connect(ClientRole::Attach { read_only: false });
        writer.send(ClientMessage::Input(b"\n".to_vec()));
        writer.output_until("done");
        assert!(!harness.server_finished(), "the holder still serves");
        harness.finish();
    }

    /// The writable attach clients are counted in the supervisor's snapshot
    /// and every change is reported to it; a read-only one does not count.
    #[test]
    fn the_supervisor_is_told_the_writable_attach_clients() {
        let harness = Harness::start("cat");
        let (writer, _) = harness.connect(ClientRole::Attach { read_only: false });
        let (_watcher, _) = harness.connect(ClientRole::Attach { read_only: true });
        let (mut supervisor, snapshot) = harness.connect(ClientRole::Supervisor);
        assert!(
            matches!(snapshot, HolderMessage::Snapshot { attached: 1, .. }),
            "{snapshot:?}"
        );

        let (_second, _) = harness.connect(ClientRole::Attach { read_only: false });
        assert_eq!(
            supervisor.recv_until(|message| matches!(message, HolderMessage::Attached(_))),
            HolderMessage::Attached(2)
        );
        drop(writer);
        assert_eq!(
            supervisor.recv_until(|message| matches!(message, HolderMessage::Attached(_))),
            HolderMessage::Attached(1)
        );
        drop(supervisor);
        harness.finish();
    }

    /// A size change resyncs every attach client with the screen reflowed to
    /// the new size, so a terminal that resized shows it at once; a resize to
    /// the current size sends nothing.
    #[test]
    fn a_size_change_resyncs_attach_clients_at_the_new_size() {
        let harness = Harness::start("cat");
        let (mut writer, _) = harness.connect(ClientRole::Attach { read_only: false });
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });

        writer.send(ClientMessage::Resize(size(30, 100)));
        for client in [&mut writer, &mut watcher] {
            let resync =
                client.recv_until(|message| matches!(message, HolderMessage::Snapshot { .. }));
            assert!(
                matches!(resync, HolderMessage::Snapshot { size: resized, .. } if resized == size(30, 100)),
                "a resync at the new size, got {resync:?}"
            );
        }

        let snapshots = writer.snapshots;
        writer.send(ClientMessage::Resize(size(30, 100)));
        writer.send(ClientMessage::Input(b"mark\n".to_vec()));
        writer.output_until("mark");
        assert_eq!(
            writer.snapshots, snapshots,
            "an unchanged size sends no resync"
        );
        harness.finish();
    }

    #[test]
    fn exit_outcome_is_held_until_collected() {
        let harness = Harness::start("exit 3");
        let child = harness.record().unwrap().child_pid();
        wait_until("the child to exit", || {
            !SignalTarget::Process(child).is_alive()
        });

        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: false });
        watcher.send(ClientMessage::CollectExit);
        assert_eq!(
            watcher.recv_until(|message| matches!(message, HolderMessage::Rejected(_))),
            HolderMessage::Rejected(RejectReason::SupervisorOnly)
        );

        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        let exited = supervisor.recv();
        assert!(
            matches!(exited, HolderMessage::Exited(ExitOutcome::Failed(_))),
            "{exited:?}"
        );
        assert!(
            !harness.server_finished(),
            "the exit is held until collected"
        );
        assert!(harness.record().is_some());

        supervisor.send(ClientMessage::CollectExit);
        wait_until("the holder to return", || harness.server_finished());
        assert!(harness.record().is_none(), "the record is removed");
        harness.finish_collected();
    }

    #[test]
    fn terminal_queries_are_answered_without_any_client() {
        let harness = Harness::start(
            r#"stty -echo -icanon min 0 time 20; printf '\033[6n'; reply=$(dd bs=1 count=6 2>/dev/null); printf 'reply=%s' "$reply" | tr '\033' E"#,
        );
        wait_until("the child to report its reply", || {
            harness.log().contains("reply=")
        });
        assert!(
            harness.log().contains("reply=E[1;1R"),
            "log: {:?}",
            harness.log()
        );
        harness.finish();
    }

    /// The background a child asks for follows the configured color scheme,
    /// so an agent themes for the terminal the user chose.
    #[test]
    fn a_background_query_is_answered_from_the_color_scheme() {
        let query = r#"stty -echo -icanon min 0 time 20; printf '\033]11;?\033\\'; reply=$(dd bs=1 count=25 2>/dev/null); printf 'reply=%s.' "$reply" | tr '\033\\' EB"#;
        let answers = [ColorScheme::Dark, ColorScheme::Light].map(|scheme| {
            let harness = Harness::start_with(query, SUPERVISOR_DEADLINE, scheme);
            wait_until("the child to report its reply", || {
                harness.log().contains("reply=")
            });
            wait_until("the whole reply", || harness.log().contains('.'));
            let log = harness.log();
            harness.finish();
            log
        });

        assert!(answers[0].contains("reply=E]11;rgb:"), "{answers:?}");
        assert!(answers[1].contains("reply=E]11;rgb:"), "{answers:?}");
        assert_ne!(answers[0], answers[1], "dark and light answer differently");
    }

    /// A shell command that waits for a line, writes `bytes` of `fill`, prints
    /// `marker`, then runs `then`.
    fn flood(bytes: usize, fill: char, marker: &str, then: &str) -> String {
        format!(r"read go; head -c {bytes} /dev/zero | tr '\0' {fill}; printf {marker}; {then}")
    }

    #[test]
    fn a_supervisor_that_never_reads_does_not_stop_the_child() {
        let harness = Harness::start(&flood(FLOOD_BYTES, 'x', "FLOOD-END", "read rest"));
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });
        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Input(b"go\n".to_vec()));

        wait_until("the flood to reach the log", || {
            harness.log().contains("FLOOD-END")
        });
        watcher.screen_until("FLOOD-END");
        drop(supervisor);
        harness.finish();
    }

    #[test]
    fn a_second_supervisor_evicts_the_first() {
        let harness = Harness::start("read x");
        let (mut first, _) = harness.connect(ClientRole::Supervisor);
        let (mut second, _) = harness.connect(ClientRole::Supervisor);

        first.expect_closed();
        second.send(ClientMessage::Control(HolderControl::Kill));
        second.recv_until(|message| matches!(message, HolderMessage::Exited(_)));
        second.send(ClientMessage::CollectExit);
        harness.finish_collected();
    }

    #[test]
    fn a_slow_attach_client_is_resynced_with_the_live_screen() {
        let harness = Harness::start(&flood(
            FLOOD_BYTES,
            'x',
            "MARK",
            "read more; printf END; read last",
        ));
        let (mut slow, _) = harness.connect(ClientRole::Attach { read_only: false });
        slow.send(ClientMessage::Input(b"go\n".to_vec()));
        wait_until("the flood to pass the stalled client", || {
            harness.log().contains("MARK")
        });

        slow.send(ClientMessage::Input(b"\n".to_vec()));
        let seen = slow.screen_until("END");
        assert!(slow.snapshots > 1, "the stalled client was resynced");
        let (mut fresh, _) = harness.connect(ClientRole::Attach { read_only: true });
        assert_eq!(fresh.screen_until("END"), seen);
        harness.finish();
    }

    #[test]
    fn input_to_a_paused_child_does_not_wedge_control() {
        let harness = Harness::start("stty raw -echo; while :; do printf tick; sleep 0.05; done");
        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Control(HolderControl::Pause));
        supervisor.send(ClientMessage::Input(vec![b'x'; STUCK_INPUT_BYTES]));
        supervisor.send(ClientMessage::Control(HolderControl::Resume));
        supervisor.send(ClientMessage::Control(HolderControl::Kill));

        supervisor.recv_until(|message| matches!(message, HolderMessage::Exited(_)));
        supervisor.send(ClientMessage::CollectExit);
        harness.finish_collected();
    }

    /// The supervisor is sent the final screen just before the exit, since the
    /// service keeps no screen of its own; an attach client is not.
    #[test]
    fn the_supervisor_gets_the_final_screen_right_before_the_exit() {
        let harness = Harness::start("read x; printf FINAL-SCREEN; exit 0");
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });
        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Input(b"go\n".to_vec()));

        let mut last = None;
        let exited = supervisor.recv_until(|message| {
            let done = matches!(message, HolderMessage::Exited(_));
            if !done {
                last = Some(message.clone());
            }
            done
        });
        supervisor.send(ClientMessage::CollectExit);
        let watcher_snapshots = watcher.snapshots;
        watcher.recv_until(|message| matches!(message, HolderMessage::Exited(_)));

        assert!(matches!(
            exited,
            HolderMessage::Exited(ExitOutcome::Succeeded)
        ));
        let Some(HolderMessage::Snapshot { repaint, size, .. }) = last else {
            panic!("the exit follows a snapshot, got {last:?}");
        };
        assert!(repaint_text(&repaint, size).contains("FINAL-SCREEN"));
        assert_eq!(watcher.snapshots, watcher_snapshots, "no snapshot at exit");
        harness.finish_collected();
    }

    #[test]
    fn attach_clients_receive_the_tail_and_exit_before_the_holder_finishes() {
        let harness = Harness::start(&flood(TAIL_BYTES, 'y', "TAIL-END", "exit 0"));
        let (mut watcher, _) = harness.connect(ClientRole::Attach { read_only: true });
        let (mut supervisor, _) = harness.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Input(b"go\n".to_vec()));
        supervisor.recv_until(|message| matches!(message, HolderMessage::Exited(_)));
        supervisor.send(ClientMessage::CollectExit);

        watcher.recv_until(|message| matches!(message, HolderMessage::Exited(_)));
        assert!(
            watcher
                .screen
                .as_ref()
                .unwrap()
                .screen_text()
                .contains("TAIL-END")
        );
        harness.finish_collected();
    }

    /// Whether `pid` still exists (an unreaped zombie counts).
    #[test]
    fn a_holder_no_supervisor_ever_reaches_kills_its_child_at_the_deadline() {
        let mut harness =
            Harness::start_with_deadline("while :; do sleep 0.05; done", SHORT_SUPERVISOR_DEADLINE);
        let child = harness.record().unwrap().child_pid();
        let (_watcher, _) = harness.connect(ClientRole::Attach { read_only: true });

        wait_until("the holder to give up", || harness.server_finished());
        harness.server.take().unwrap().join().unwrap().unwrap();
        assert!(harness.record().is_none(), "the record is removed");
        assert!(!harness.config.socket().exists(), "the socket is removed");
        wait_until("the child to be killed", || {
            !SignalTarget::Process(child).is_alive()
        });
    }

    #[test]
    fn a_holder_that_had_a_supervisor_outlives_its_deadline() {
        let harness = Harness::start_with_deadline("read x", SHORT_SUPERVISOR_DEADLINE);
        let (supervisor, _) = harness.connect(ClientRole::Supervisor);
        drop(supervisor);

        thread::sleep(SHORT_SUPERVISOR_DEADLINE * 3);
        assert!(!harness.server_finished(), "the holder carries on");
        assert!(harness.record().is_some());
        harness.finish();
    }

    /// A config whose socket lives in a directory nothing has created yet, so
    /// whether it exists afterwards shows whether the holder ever bound.
    fn unbound_config(dir: &Path, working_dir: PathBuf, log: PathBuf) -> HolderConfig {
        let request = test_request(
            "dev server",
            "true",
            Some(dir.to_path_buf()),
            Some(working_dir),
        );
        HolderConfig::builder()
            .id(HolderId::generate().unwrap())
            .request(request)
            .socket(dir.join(SOCKET_DIR).join(SOCKET_FILE))
            .record_dir(dir.join("records"))
            .log(log)
            .supervisor_deadline(SUPERVISOR_DEADLINE)
            .build()
    }

    /// A holder whose log cannot open or whose child cannot spawn fails
    /// before binding, so an accepting socket always means a serving holder.
    #[test]
    fn a_holder_that_fails_to_start_never_binds_its_socket() {
        let scratch = scratch();
        let dir = scratch.path();
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, b"").unwrap();
        let bad_log = unbound_config(dir, dir.to_path_buf(), blocker.join("dev.log"));
        assert!(matches!(
            HolderServer::run(bad_log),
            Err(HolderError::Io(_))
        ));
        assert!(
            !dir.join(SOCKET_DIR).exists(),
            "a bad log fails before binding"
        );

        let missing_dir = unbound_config(dir, dir.join("gone"), dir.join("logs").join("dev.log"));
        assert!(matches!(
            HolderServer::run(missing_dir),
            Err(HolderError::Pty(PtyError::InvalidWorkingDir(_)))
        ));
        assert!(
            !dir.join(SOCKET_DIR).exists(),
            "a failed spawn fails before binding"
        );
    }

    #[test]
    fn the_socket_is_private_to_its_owner() {
        let harness = Harness::start("read x");
        let socket = harness.config.socket();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode(socket), EXPECTED_SOCKET_MODE);
        assert_eq!(mode(socket.parent().unwrap()), EXPECTED_DIR_MODE);
        harness.finish();
    }

    #[test]
    fn a_socket_directory_other_users_can_enter_is_refused() {
        let scratch = scratch();
        fs::set_permissions(scratch.path(), Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        let socket = scratch.path().join(SOCKET_FILE);

        assert!(matches!(
            bind_private(&socket),
            Err(HolderError::InsecureSocketDir(_))
        ));
        assert!(!socket.exists());
    }

    #[test]
    fn the_record_guard_removes_the_record_on_any_exit() {
        let scratch = scratch();
        let dir = scratch.path().join("records");
        let record = HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(scratch.path().to_path_buf()).unwrap())
            .process(ProcessName::try_new("dev server").unwrap())
            .command(CommandLine::try_new("true").unwrap())
            .stop(test_stop())
            .socket(scratch.path().join(SOCKET_FILE))
            .holder_pid(Pid::try_new(process::id()).unwrap())
            .child_pid(Pid::try_new(FAKE_PID).unwrap())
            .child_start_token(None)
            .log(scratch.path().join("dev.log"))
            .child_started_at(SystemTime::now())
            .build();
        record.save(&dir).unwrap();

        drop(RecordGuard {
            record: record.clone(),
            dir: dir.clone(),
            removed: false,
        });
        assert!(HolderRecord::load_all(&dir).is_empty(), "dropped unremoved");

        record.save(&dir).unwrap();
        RecordGuard {
            record,
            dir: dir.clone(),
            removed: false,
        }
        .remove()
        .unwrap();
        assert!(
            HolderRecord::load_all(&dir).is_empty(),
            "removed explicitly"
        );
    }

    /// A server around a [`FakeChild`], with the flag its kill sets.
    fn fake_server(scratch: &TempDir) -> (HolderServer, Arc<AtomicBool>) {
        let (server, killed, _) = recording_server(scratch);
        (server, killed)
    }

    /// As [`fake_server`], with what the child's input received too.
    fn recording_server(scratch: &TempDir) -> (HolderServer, Arc<AtomicBool>, Arc<Mutex<Vec<u8>>>) {
        let log = RotatingLog::open(
            scratch.path().join("dev.log"),
            ByteSize::try_new(RawByteSize::mib(1)).unwrap(),
        )
        .unwrap();
        let killed = Arc::new(AtomicBool::new(false));
        let written = Arc::new(Mutex::new(Vec::new()));
        let child = FakeChild {
            killed: Arc::clone(&killed),
            written: Arc::clone(&written),
        };
        let server = HolderServer::new(
            Box::new(child),
            log,
            bounded(1),
            &request(scratch.path(), "true", ColorScheme::Dark),
        )
        .unwrap();
        (server, killed, written)
    }

    /// Output the emulator panics on rebuilds the screen instead of taking
    /// the holder, and so the child, down, and warns the supervisor.
    #[test]
    fn an_emulator_panic_rebuilds_the_screen_and_keeps_serving() {
        let scratch = scratch();
        let (mut server, killed) = fake_server(&scratch);
        server.emulator =
            TerminalEmulator::one_column(Rows::try_new(ROWS).unwrap(), 0, ColorScheme::Dark);
        let (supervisor, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(WAIT)).unwrap();
        server.on_connected(ClientId(0), supervisor);
        server
            .on_hello(ClientId(0), ClientRole::Supervisor)
            .unwrap();

        server
            .on_output("wide \u{754c}".as_bytes().to_vec())
            .unwrap();
        server.on_output(b"after".to_vec()).unwrap();

        assert!(!killed.load(Ordering::SeqCst), "the child keeps running");
        assert!(!server.emulator.screen_text().contains('\u{754c}'));
        assert!(
            server.emulator.screen_text().contains('r'),
            "later output is shown"
        );
        while !matches!(
            HolderMessage::read_from(&mut peer).unwrap(),
            HolderMessage::Warning(_)
        ) {}
    }

    /// The chunk the emulator panics on still reaches the keyboard tracker,
    /// and the query it carries is still answered.
    #[test]
    fn the_chunk_an_emulator_panics_on_still_drives_the_keyboard_protocol() {
        let scratch = scratch();
        let (mut server, _, written) = recording_server(&scratch);
        server.emulator =
            TerminalEmulator::one_column(Rows::try_new(ROWS).unwrap(), 0, ColorScheme::Dark);

        server
            .on_output("\x1b[>1u\x1b[?uwide \u{754c}".as_bytes().to_vec())
            .unwrap();

        assert_ne!(
            server.keyboard.protocol(),
            KittyKeyboardTracker::default().protocol(),
            "the pushed keyboard level is tracked"
        );
        wait_until("the keyboard query to be answered", || {
            String::from_utf8_lossy(&written.lock().unwrap()).contains("\x1b[?1u")
        });
    }

    #[test]
    fn a_refused_resize_leaves_the_size_and_screen_unchanged() {
        let scratch = scratch();
        let (mut server, _) = fake_server(&scratch);

        assert!(server.apply_size(size(30, 100)).is_err());
        assert_eq!(server.size, size(ROWS, COLS));
        assert_eq!(server.emulator.size(), (ROWS, COLS));
    }

    /// A holder that stops serving early, as on a `serve` error, kills a child
    /// it can no longer reach; one whose child already exited leaves it be.
    #[test]
    fn a_server_dropped_before_its_child_exits_kills_the_child() {
        let scratch = scratch();
        let (server, killed) = fake_server(&scratch);
        drop(server);
        assert!(killed.load(Ordering::SeqCst), "a running child is killed");

        let (mut server, killed) = fake_server(&scratch);
        server.on_exit(ExitOutcome::Succeeded).unwrap();
        drop(server);
        assert!(
            !killed.load(Ordering::SeqCst),
            "an exited child is not signalled"
        );
    }
}
