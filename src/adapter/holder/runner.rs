use std::{
    fs, io,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    client_message::ClientMessage,
    client_role::ClientRole,
    config::{HolderConfig, is_handover_file},
    dirs::HolderDirs,
    error::HolderError,
    handle::HolderHandle,
    holder_control::HolderControl,
    holder_launcher::HolderLauncher,
    id::HolderId,
    protocol::HolderMessage,
    record::HolderRecord,
};
use crate::{
    adapter::{
        ipc, process_identity::LocalProcessIdentity, pty::ensure_working_dir,
        signal_target::SignalTarget,
    },
    domain::{
        port::{OutputSink, ProcessHandle, ProcessRunner},
        pty::{PtyError, SpawnRequest},
        value::{Pid, PositiveDuration, ProjectRoot},
    },
};

/// Longest a launched holder may take to accept its supervisor and send its
/// screen, and longest an adopted one may take to send its screen.
pub const HOLDER_READY_TIMEOUT: PositiveDuration =
    match PositiveDuration::try_new(Duration::from_secs(5)) {
        Ok(timeout) => timeout,
        Err(_) => panic!("the holder ready timeout must be positive"),
    };
/// Multiple of the ready timeout a launched holder waits for its first
/// supervisor. Above one, and the ready timeout is positive, so a holder the
/// runner is still waiting for never gives up, and one the runner gave up on
/// always does.
const SUPERVISOR_DEADLINE_FACTOR: u32 = 2;
/// Interval between attempts to reach a launching holder's socket.
const READY_POLL: Duration = Duration::from_millis(10);
/// Longest the runner spends stopping a holder it gave up on.
const ABANDON_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest a probe of whether a holder still accepts connections waits.
const LIVENESS_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Runs every process under its own detached holder, so processes outlive the
/// service, and finds and adopts the holders a previous service left running.
pub struct HolderRunner {
    /// Starts each holder.
    launcher: Box<dyn HolderLauncher>,
    /// Where holders keep their sockets, records and logs.
    dirs: HolderDirs,
    /// Longest a launched holder may take to serve its supervisor.
    ready_timeout: PositiveDuration,
}

impl HolderRunner {
    /// Where this runner's holders keep their sockets, records and logs.
    pub fn dirs(&self) -> &HolderDirs {
        &self.dirs
    }

    /// Creates a runner launching holders through `launcher` into `dirs`. Each
    /// request carries its own launch settings: size, scrollback, log cap and
    /// colors.
    pub fn new(launcher: Box<dyn HolderLauncher>, dirs: HolderDirs) -> Self {
        Self {
            launcher,
            dirs,
            ready_timeout: HOLDER_READY_TIMEOUT,
        }
    }

    /// How long a launched holder waits for its first supervisor: past the
    /// ready timeout, so a holder the runner still waits for never gives up.
    fn supervisor_deadline(&self) -> Duration {
        self.ready_timeout
            .into_inner()
            .saturating_mul(SUPERVISOR_DEADLINE_FACTOR)
    }

    /// The record files that cannot be read, which no service can adopt.
    pub fn unreadable_records(&self) -> Vec<PathBuf> {
        HolderRecord::unreadable(self.dirs.records())
    }

    /// Deletes the spec and error files left in the socket directory by
    /// holders that never started or whose failure no runner read. A spec
    /// carries the child's environment, so it must not linger; only the
    /// runner waiting on a launch reads them, and none is waiting when this
    /// runs at service start. A file that cannot be removed, or an entry that
    /// cannot be read, does not stop the sweep; each is returned with why, an
    /// unreadable entry under the directory's own path.
    ///
    /// # Errors
    /// Returns the I/O error from listing the directory; an absent directory
    /// holds nothing to sweep.
    pub fn sweep_handover_files(&self) -> io::Result<Vec<(PathBuf, io::Error)>> {
        let entries = match fs::read_dir(self.dirs.sockets()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let sockets = self.dirs.sockets();
        Ok(entries
            .filter_map(|entry| match entry {
                Ok(entry) => {
                    let path = entry.path();
                    is_handover_file(&path)
                        .then(|| fs::remove_file(&path).err().map(|error| (path, error)))
                        .flatten()
                },
                Err(error) => Some((sockets.clone(), error)),
            })
            .collect())
    }

    /// Shortens the ready timeout, so a test can outlast it quickly.
    #[cfg(test)]
    fn with_ready_timeout(mut self, ready_timeout: PositiveDuration) -> Self {
        self.ready_timeout = ready_timeout;
        self
    }

    /// Becomes the supervisor of the running holder `record` describes,
    /// forwarding its screen and then its output and exit to `sink`.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be reached or sends no
    /// screen.
    pub fn adopt(
        &self,
        record: &HolderRecord,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        let stream = UnixStream::connect(record.socket())?;
        let handle = HolderHandle::supervise(
            stream,
            record.socket(),
            sink,
            HOLDER_READY_TIMEOUT.into_inner(),
        )?
        .with_started_at(record.child_started_at());
        Ok(Box::new(handle))
    }

    /// The records worth adopting, deleting every other record: those of
    /// holders still running, and those of holders that died while their
    /// child, the same process still, runs on, so it is adopted as
    /// unsupervised rather than started a second time. A holder runs while
    /// its pid is alive and its socket accepts a connection; a reused pid
    /// fails the second test and a holder holding an uncollected exit passes
    /// both.
    pub fn live_records(&self) -> Vec<HolderRecord> {
        HolderRecord::load_all(self.dirs.records())
            .into_iter()
            .filter(|record| {
                let kept = is_live(record) || child_runs(record);
                if !kept {
                    let _ = record.remove(self.dirs.records());
                }
                kept
            })
            .collect()
    }

    /// The record of the holder supervising the child with `child_pid`, naming
    /// its socket and log. A live holder's record wins over a stale one left
    /// for the same pid, then one whose child outlived its holder.
    pub fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        let candidates: Vec<HolderRecord> = HolderRecord::load_all(self.dirs.records())
            .into_iter()
            .filter(|record| record.child_pid() == child_pid)
            .collect();
        let live = candidates.iter().position(is_live);
        let surviving = || candidates.iter().position(child_runs);
        live.or_else(surviving)
            .and_then(|index| candidates.into_iter().nth(index))
    }

    /// Deletes the holder sockets no running holder serves: what a holder that
    /// died without unlinking its socket left behind. A socket is kept while a
    /// record names it and that record's holder process is alive, and while
    /// it is younger than a holder's first-supervisor deadline, since a holder
    /// binds its socket just before it writes its record and one never
    /// supervised is gone by then. How a connection to a dead socket fails
    /// differs by platform, so it is never the test. Run beside
    /// [`Self::sweep_handover_files`] at service start. A file that cannot be
    /// removed is returned with why.
    ///
    /// # Errors
    /// Returns the I/O error from listing the directory; an absent directory
    /// holds nothing to sweep.
    pub fn sweep_orphan_sockets(&self) -> io::Result<Vec<(PathBuf, io::Error)>> {
        let entries = match fs::read_dir(self.dirs.sockets()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let served: Vec<PathBuf> = HolderRecord::load_all(self.dirs.records())
            .into_iter()
            .filter(holder_runs)
            .map(|record| record.socket().clone())
            .collect();
        let settled = SystemTime::now()
            .checked_sub(self.supervisor_deadline())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        Ok(entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| self.dirs.is_socket(path) && !served.contains(path))
            .filter(|path| modified_before(path, settled))
            .filter_map(|path| fs::remove_file(&path).err().map(|error| (path, error)))
            .collect())
    }
}

impl ProcessRunner for HolderRunner {
    /// Launches a holder for `request`, waits up to [`HOLDER_READY_TIMEOUT`]
    /// for it to serve, and supervises it. A request the holder would refuse
    /// fails here, before anything is launched. A holder that reports its own
    /// failure is left to finish; any other failure abandons it, and a reason
    /// it reports meanwhile replaces the runner's own error. A failure takes at
    /// most the ready timeout plus the abandon timeout (2 s) to return.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the working directory is invalid, the
    /// request names no project, the socket directory is not private, or the
    /// holder cannot be launched or does not become ready.
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        ensure_working_dir(&request)?;
        let Some(project) = request
            .project()
            .clone()
            .and_then(|root| ProjectRoot::try_new(root).ok())
        else {
            return Err(HolderError::MissingProject.into());
        };
        let log = self.dirs.process_log(&project, request.process());
        self.dirs.prepare_sockets()?;
        let id = HolderId::generate().map_err(|error| PtyError::System(error.to_string()))?;
        let socket = self.dirs.socket_path(&id);
        let config = HolderConfig::builder()
            .log(log)
            .id(id)
            .request(request)
            .socket(socket.clone())
            .record_dir(self.dirs.records().clone())
            .supervisor_deadline(self.supervisor_deadline())
            .build();
        let error_file = config.error_path();
        self.launcher.launch(config)?;
        let supervised = match self.supervise_launched(&socket, &error_file, sink) {
            Err(error @ HolderError::FailedToStart { .. }) => Err(error),
            Err(error) => {
                abandon(&socket);
                Err(reported_failure(&socket, &error_file).unwrap_or(error))
            },
            Ok(handle) => Ok(handle),
        };
        let _ = fs::remove_file(&error_file);
        Ok(Box::new(supervised?))
    }
}

impl HolderRunner {
    /// Becomes the supervisor of a holder just launched at `socket`, within
    /// the ready timeout.
    ///
    /// # Errors
    /// Returns [`HolderError::FailedToStart`] with the holder's own error when
    /// it reports one, [`HolderError::NotReady`] when it does not serve in
    /// time, or the error from greeting it.
    fn supervise_launched(
        &self,
        socket: &Path,
        error_file: &Path,
        sink: Box<dyn OutputSink>,
    ) -> Result<HolderHandle, HolderError> {
        let deadline = Instant::now() + self.ready_timeout.into_inner();
        let stream = connect_by(socket, error_file, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        HolderHandle::supervise(stream, socket, sink, remaining.max(READY_POLL))
    }
}

/// Stops a holder the runner gave up on: retries reaching it, then supervises
/// it, kills its child and collects the exit, so it finishes, all within
/// [`ABANDON_TIMEOUT`]. A holder still unreachable then stops itself at its
/// supervisor deadline.
pub(crate) fn abandon(socket: &Path) {
    let deadline = Instant::now() + ABANDON_TIMEOUT;
    loop {
        match UnixStream::connect(socket) {
            Ok(mut stream) => {
                let _ = kill_and_collect(&mut stream, socket, deadline);
                return;
            },
            Err(_) if Instant::now() >= deadline => return,
            Err(_) => thread::sleep(READY_POLL),
        }
    }
}

/// Supervises the holder on `stream`, kills its child and collects the exit,
/// all before `deadline`.
///
/// # Errors
/// Returns a [`HolderError`] when the exchange fails or the exit does not
/// arrive before the deadline.
fn kill_and_collect(
    stream: &mut UnixStream,
    socket: &Path,
    deadline: Instant,
) -> Result<(), HolderError> {
    let remaining = || {
        Some(deadline.saturating_duration_since(Instant::now()))
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| HolderError::NotReady(socket.to_path_buf()))
    };
    stream.set_write_timeout(Some(remaining()?))?;
    ClientMessage::Hello {
        role: ClientRole::Supervisor,
    }
    .write_to(stream)?;
    ClientMessage::Control(HolderControl::Kill).write_to(stream)?;
    loop {
        stream.set_read_timeout(Some(remaining()?))?;
        if matches!(HolderMessage::read_from(stream)?, HolderMessage::Exited(_)) {
            break;
        }
    }
    ClientMessage::CollectExit.write_to(stream)
}

/// Whether the holder `record` describes is still running; see
/// [`HolderRunner::live_records`]. A probe that times out finds a listener
/// too busy to accept, which is a running holder, never a stale one.
fn is_live(record: &HolderRecord) -> bool {
    holder_runs(record)
        && ipc::connect_within(record.socket(), LIVENESS_PROBE_TIMEOUT)
            .map_or_else(|error| error.kind() == io::ErrorKind::TimedOut, |_| true)
}

/// Whether the holder process `record` names is alive.
fn holder_runs(record: &HolderRecord) -> bool {
    SignalTarget::Process(record.holder_pid()).is_alive()
}

/// Whether the child `record` names still runs as the same process, as one
/// that ignores the hangup does after its holder died.
fn child_runs(record: &HolderRecord) -> bool {
    LocalProcessIdentity::is_same_process(record.child_pid(), record.child_start_token())
}

/// Whether the file at `path` was last modified before `instant`. One whose
/// time cannot be read counts as recent, so it is kept.
fn modified_before(path: &Path, instant: SystemTime) -> bool {
    fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified < instant)
}

/// Connects to a launching holder's socket, retrying while it is not bound
/// yet, until `deadline`. Stops early when the holder reports in `error_file`
/// that it failed, and reads that file once more when the deadline passes.
///
/// # Errors
/// Returns [`HolderError::FailedToStart`] with the holder's error,
/// [`HolderError::NotReady`] when the deadline passes first, or an I/O error
/// other than the socket being absent or not yet accepting.
fn connect_by(
    socket: &Path,
    error_file: &Path,
    deadline: Instant,
) -> Result<UnixStream, HolderError> {
    loop {
        if let Some(failure) = reported_failure(socket, error_file) {
            return Err(failure);
        }
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if !matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Err(error.into());
            },
            Err(_) if Instant::now() >= deadline => {
                return Err(reported_failure(socket, error_file)
                    .unwrap_or_else(|| HolderError::NotReady(socket.to_path_buf())));
            },
            Err(_) => thread::sleep(READY_POLL),
        }
    }
}

/// The failure the holder for `socket` reported in `error_file`, if it has
/// written one.
fn reported_failure(socket: &Path, error_file: &Path) -> Option<HolderError> {
    fs::read_to_string(error_file)
        .ok()
        .filter(|reason| !reason.is_empty())
        .map(|reason| HolderError::FailedToStart {
            socket: socket.to_path_buf(),
            reason,
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs, mem,
        os::unix::net::{UnixDatagram, UnixListener},
        path::PathBuf,
        process,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
    };

    use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
    use rustix::{net::listen, process::Signal};
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            holder::{
                HolderServer,
                client::{greet, screen_text},
                error_path_beside, run_holder,
                testing::{TEST_COLS, TEST_ROWS, TEST_SCROLLBACK, test_request, test_stop},
            },
            terminal::TerminalEmulator,
            test_scratch::short_scratch,
        },
        domain::{
            color_scheme::ColorScheme,
            process::StopSignal,
            pty::{ExitCause, ExitOutcome, ProcessOutput, PtySize, ScreenSource},
            value::{Cols, CommandLine, ProcessName, Rows, StartToken},
        },
    };

    /// Bound on every wait in these tests; reaching it fails the test.
    const WAIT: Duration = Duration::from_secs(10);
    /// Interval between checks of a polled condition.
    const POLL: Duration = Duration::from_millis(10);
    /// Longest one screen probe waits for the holder, far below [`WAIT`] so a
    /// polling loop stays within its bound.
    const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "mr-";
    /// PTY rows of every test child.
    const ROWS: u16 = TEST_ROWS;
    /// PTY cols of every test child.
    const COLS: u16 = TEST_COLS;
    /// Numbered lines the heavy-output child prints.
    const HEAVY_LINES: usize = 20_000;
    /// Line the heavy-output child must have reached before the runner drops.
    const HEAVY_DROP_AT: &str = "line-2000";
    /// The heavy-output child's last line, whole, so its scroll has happened:
    /// waiting for `DONE` alone can see the text before the newline that
    /// scrolls the screen.
    const HEAVY_DONE_LINE: &str = "DONE\r\n";
    /// Lines between the heavy-output child's short pauses, which stretch its
    /// output across the supervisor's departure and the adoption.
    const HEAVY_PAUSE_EVERY: usize = 1000;
    /// Length of each pause, in seconds as `sleep` reads it.
    const HEAVY_PAUSE: &str = "0.05";
    /// Stale records saved beside a live one for the same child, after it,
    /// so a directory listing is unlikely to name the live one first.
    const STALE_RECORDS: usize = 4;
    /// Grace given to a child that ignores the stop signal.
    const STOP_GRACE: Duration = Duration::from_millis(500);
    /// Ready timeout of a runner a test makes give up quickly.
    const SHORT_READY: PositiveDuration =
        match PositiveDuration::try_new(Duration::from_millis(300)) {
            Ok(timeout) => timeout,
            Err(_) => panic!("the short ready timeout must be positive"),
        };
    /// How long a slow holder takes to start: past [`SHORT_READY`].
    const SLOW_START: Duration = Duration::from_millis(600);
    /// How long a holder an abandon races takes to start: well inside
    /// [`ABANDON_TIMEOUT`].
    const STARTING_DELAY: Duration = Duration::from_millis(300);
    /// A first-supervisor deadline past any test's length.
    const LONG_SUPERVISOR_DEADLINE: Duration = Duration::from_secs(60);
    /// A regular file the bad-log test makes the log directory's parent.
    const NOT_A_DIRECTORY: &str = "not-a-dir";
    /// A spec no holder can parse.
    const CORRUPT_SPEC: &[u8] = b"{not a spec";
    /// A handover name the sweep cannot remove, made a directory.
    const STUCK_HANDOVER: &str = "stuck.json";
    /// File the pid-reporting child writes its pid to.
    const PID_FILE: &str = "child.pid";
    /// Time a surviving child would need to write its pid file late.
    const PID_FILE_GRACE: Duration = Duration::from_millis(500);

    /// Every holder thread a fixture's runners started.
    type Holders = Arc<Mutex<Vec<JoinHandle<Result<(), HolderError>>>>>;

    /// Runs each holder on a thread of the test process, after `delay`.
    struct ThreadHolderLauncher {
        holders: Holders,
        delay: Duration,
    }

    impl HolderLauncher for ThreadHolderLauncher {
        fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
            let delay = self.delay;
            let holder = thread::spawn(move || {
                thread::sleep(delay);
                HolderServer::run(config)
            });
            self.holders.lock().unwrap().push(holder);
            Ok(())
        }
    }

    /// How a [`SpecLauncher`] breaks the holder it starts.
    enum Sabotage {
        /// An unreadable spec, as a service and holder binary that disagree
        /// on its shape would hand over.
        CorruptSpec,
        /// The child's working directory disappears after the runner checked
        /// it, so the PTY spawn fails.
        RemoveDir(PathBuf),
    }

    /// Hands each holder its spec file and runs it through the real entry
    /// point on a thread, sabotaged first.
    struct SpecLauncher {
        holders: Holders,
        sabotage: Option<Sabotage>,
    }

    impl HolderLauncher for SpecLauncher {
        fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
            let spec = config.spec_path();
            match &self.sabotage {
                Some(Sabotage::CorruptSpec) => fs::write(&spec, CORRUPT_SPEC).unwrap(),
                Some(Sabotage::RemoveDir(dir)) => {
                    config.write_private(&spec).unwrap();
                    fs::remove_dir(dir).unwrap();
                },
                None => config.write_private(&spec).unwrap(),
            }
            let holder = thread::spawn(move || run_holder(&spec));
            self.holders.lock().unwrap().push(holder);
            Ok(())
        }
    }

    /// Forwards a holder handle's output into a channel.
    struct ChannelSink(Sender<ProcessOutput>);

    impl OutputSink for ChannelSink {
        fn send(&self, output: ProcessOutput) {
            let _ = self.0.send(output);
        }
    }

    /// A sink whose receiver is gone: it notes that the exit arrived and
    /// delivers nothing.
    struct LosingSink(Arc<AtomicBool>);

    impl OutputSink for LosingSink {
        fn send(&self, output: ProcessOutput) {
            if matches!(output, ProcessOutput::Exited(_)) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
    }

    /// What a sink received: each chunk, the exit, and the screen they draw.
    struct Watch {
        outputs: Receiver<ProcessOutput>,
        chunks: Vec<Vec<u8>>,
        exit: Option<ExitOutcome>,
        screen: TerminalEmulator,
    }

    impl Watch {
        fn new() -> (Box<dyn OutputSink>, Self) {
            let (sender, outputs) = unbounded();
            let watch = Self {
                outputs,
                chunks: Vec::new(),
                exit: None,
                screen: TerminalEmulator::new(
                    Rows::try_new(ROWS).unwrap(),
                    Cols::try_new(COLS).unwrap(),
                    TEST_SCROLLBACK,
                    ColorScheme::Dark,
                ),
            };
            (Box::new(ChannelSink(sender)), watch)
        }

        /// Receives until `condition` holds, failing the test at [`WAIT`].
        fn until(&mut self, what: &str, condition: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + WAIT;
            while !condition(self) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match self.outputs.recv_timeout(remaining) {
                    Ok(output) => self.apply(output),
                    Err(_) => panic!("timed out waiting for {what}"),
                }
            }
        }

        /// Applies everything received so far without waiting.
        fn drain(&mut self) {
            while let Ok(output) = self.outputs.try_recv() {
                self.apply(output);
            }
        }

        fn apply(&mut self, output: ProcessOutput) {
            match output {
                ProcessOutput::Chunk(bytes)
                | ProcessOutput::Placed { bytes, .. }
                | ProcessOutput::Repaint { bytes, .. } => {
                    self.screen.process(&bytes);
                    self.chunks.push(bytes);
                },
                ProcessOutput::Exited(outcome) => self.exit = Some(outcome),
                ProcessOutput::Warning(_) | ProcessOutput::Attached(_) => {},
                ProcessOutput::Unreadable(reason) => panic!("unreadable output: {reason}"),
            }
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.chunks.concat()).into_owned()
        }

        /// Paints the screen `handle` connected with first, as the service
        /// does, since it is handed back rather than sent to the sink.
        fn connected(&mut self, handle: &mut Box<dyn ProcessHandle>) {
            let screen = handle
                .take_initial_screen()
                .expect("a holder connection carries its screen");
            self.apply(ProcessOutput::Chunk(screen));
        }
    }

    /// Polls `condition` until it holds, failing the test at [`WAIT`].
    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
            thread::sleep(POLL);
        }
    }

    /// Whether the holder at `socket` serves its screen at `size`. A holder
    /// applies one connection's frames in order, so once it serves at a size a
    /// supervisor sent, it has applied everything that supervisor sent before.
    fn serves_at(socket: &Path, size: PtySize) -> bool {
        let Ok(mut stream) = UnixStream::connect(socket) else {
            return false;
        };
        greet(
            &mut stream,
            socket,
            ClientRole::Attach { read_only: true },
            PROBE_TIMEOUT,
        )
        .is_ok_and(|greeting| greeting.size() == size)
    }

    /// Holder dirs in a scratch directory, plus every holder thread started
    /// there; dropping it kills and collects every child left running.
    struct Fixture {
        dirs: HolderDirs,
        holders: Holders,
        scratch: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let scratch = short_scratch(SCRATCH_PREFIX);
            let root = scratch.path();
            let dirs = HolderDirs::builder()
                .sockets(root.join("sock"))
                .records(root.join("records"))
                .logs(root.join("logs"))
                .build();
            Self {
                dirs,
                holders: Arc::default(),
                scratch,
            }
        }

        /// A fresh runner over the same dirs, as a restarted service has.
        fn runner(&self) -> HolderRunner {
            self.runner_with(Box::new(ThreadHolderLauncher {
                holders: Arc::clone(&self.holders),
                delay: Duration::ZERO,
            }))
        }

        fn runner_with(&self, launcher: Box<dyn HolderLauncher>) -> HolderRunner {
            self.runner_over(self.dirs.clone(), launcher)
        }

        fn runner_over(&self, dirs: HolderDirs, launcher: Box<dyn HolderLauncher>) -> HolderRunner {
            HolderRunner::new(launcher, dirs)
        }

        /// The project every test child belongs to: the scratch directory.
        fn root(&self) -> ProjectRoot {
            ProjectRoot::try_new(self.scratch.path().to_path_buf()).unwrap()
        }

        /// A launcher running holders from spec files, sabotaged as given.
        fn spec_launcher(&self, sabotage: Option<Sabotage>) -> Box<dyn HolderLauncher> {
            Box::new(SpecLauncher {
                holders: Arc::clone(&self.holders),
                sabotage,
            })
        }

        /// A holder config for `command` as the runner would build it, with
        /// `supervisor_deadline`.
        fn config(&self, command: &str, supervisor_deadline: Duration) -> HolderConfig {
            let id = HolderId::generate().unwrap();
            let request = self.request(command);
            HolderConfig::builder()
                .log(self.dirs.process_log(&self.root(), request.process()))
                .socket(self.dirs.socket_path(&id))
                .id(id)
                .request(request)
                .record_dir(self.dirs.records().clone())
                .supervisor_deadline(supervisor_deadline)
                .build()
        }

        /// A request to run `command` as `dev server` of this fixture's
        /// project.
        fn request(&self, command: &str) -> SpawnRequest {
            test_request(
                "dev server",
                command,
                Some(self.scratch.path().to_path_buf()),
                None,
            )
        }

        fn record_files(&self) -> Vec<HolderRecord> {
            HolderRecord::load_all(self.dirs.records())
        }

        fn log(&self, record: &HolderRecord) -> String {
            String::from_utf8_lossy(&fs::read(record.log()).unwrap_or_default()).into_owned()
        }
    }

    impl Drop for Fixture {
        /// Kills every child still recorded, collects its exit so its holder
        /// returns, and joins every holder thread, all bounded by [`WAIT`].
        /// A record naming the test process itself (a stale-record test's
        /// stand-in) is never signalled.
        fn drop(&mut self) {
            for record in self.record_files() {
                if record.child_pid().into_inner() != process::id() {
                    let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
                }
                abandon(record.socket());
            }
            let started = Instant::now();
            let holders = mem::take(&mut *self.holders.lock().unwrap());
            for holder in holders {
                while !holder.is_finished() && started.elapsed() < WAIT {
                    thread::sleep(POLL);
                }
                if holder.is_finished() {
                    let _ = holder.join();
                }
            }
        }
    }

    #[test]
    fn spawned_process_streams_output_through_the_sink() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(fixture.request("printf hello; read x"), sink)
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the child's output", |watch| watch.text().contains("hello"));

        let pid = handle.process_id().unwrap();
        let record = runner.locate(pid).unwrap();
        assert_eq!(
            record.socket().parent(),
            Some(fixture.dirs.sockets().as_path())
        );
        assert!(fixture.log(&record).contains("hello"));

        handle.write_input(b"\n").unwrap();
        watch.until("the child's exit", |watch| watch.exit.is_some());
        assert_eq!(watch.exit, Some(ExitOutcome::Succeeded));
        handle.acknowledge_exit().unwrap();
        wait_until("the holder to finish once the exit is collected", || {
            fixture.record_files().is_empty()
        });
    }

    /// An exit the supervisor never received stays with the holder: its record
    /// survives and a later adoption still sees the exit, which only an
    /// acknowledgement lets go of.
    #[test]
    fn an_exit_the_supervisor_never_handled_is_held_for_a_later_adoption() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let reported = Arc::new(AtomicBool::new(false));
        let mut handle = runner
            .spawn(
                fixture.request("read x"),
                Box::new(LosingSink(Arc::clone(&reported))),
            )
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();

        handle.write_input(b"\n").unwrap();
        wait_until("the holder to report the exit", || {
            reported.load(Ordering::SeqCst)
        });
        let probe = PtySize::try_new(ROWS + 1, COLS).unwrap();
        handle
            .resize(probe)
            .expect("the holder keeps serving its supervisor after the exit");
        wait_until("the holder to apply the resize sent after the exit", || {
            serves_at(record.socket(), probe)
        });
        drop(handle);
        assert_eq!(
            fixture.record_files(),
            vec![record.clone()],
            "the holder keeps the unacknowledged exit"
        );

        let runner = fixture.runner();
        assert_eq!(runner.live_records(), vec![record.clone()]);
        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&record, sink).unwrap();
        watch.until("the held exit", |watch| watch.exit.is_some());
        assert_eq!(watch.exit, Some(ExitOutcome::Succeeded));
        adopted.acknowledge_exit().unwrap();
        wait_until("the holder to finish once the exit is acknowledged", || {
            fixture.record_files().is_empty()
        });
    }

    /// An acknowledgement reaches the holder even when the handle is dropped
    /// straight after it, so the holder finishes and removes its record.
    #[test]
    fn an_acknowledged_exit_lets_the_holder_finish_when_the_handle_drops_at_once() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner.spawn(fixture.request("read x"), sink).unwrap();

        handle.write_input(b"\n").unwrap();
        watch.until("the child's exit", |watch| watch.exit.is_some());
        handle.acknowledge_exit().unwrap();
        drop(handle);

        wait_until("the holder to finish and remove its record", || {
            fixture.record_files().is_empty()
        });
    }

    #[test]
    fn dropping_the_runner_leaves_the_process_running_and_adopt_resumes_output() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request("while :; do echo tick; sleep 0.05; done"),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("a tick", |watch| watch.text().contains("tick"));
        let pid = handle.process_id().unwrap();
        drop(handle);
        drop(runner);

        let runner = fixture.runner();
        let records = runner.live_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].child_pid(), pid);

        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&records[0], sink).unwrap();
        watch.connected(&mut adopted);
        assert_eq!(adopted.process_id(), Some(pid));
        watch.until("a tick after the adoption snapshot", |watch| {
            watch
                .chunks
                .iter()
                .skip(1)
                .any(|chunk| String::from_utf8_lossy(chunk).contains("tick"))
        });
        assert!(watch.exit.is_none(), "the adopted child is still running");

        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit.is_some());
    }

    #[test]
    fn adoption_after_heavy_output_rebuilds_the_exact_screen_and_log_has_no_gap() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let command = format!(
            "i=0; while [ $i -lt {HEAVY_LINES} ]; do echo line-$i; i=$((i+1)); \
             [ $((i % {HEAVY_PAUSE_EVERY})) -eq 0 ] && sleep {HEAVY_PAUSE}; done; \
             read go; echo DONE; read x"
        );
        let handle = runner.spawn(fixture.request(&command), sink).unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        wait_until("the output to be under way", || {
            fixture.log(&record).contains(HEAVY_DROP_AT)
        });
        drop(handle);
        drop(runner);
        assert!(
            !fixture.log(&record).contains("DONE"),
            "the supervisor leaves mid-stream"
        );

        let runner = fixture.runner();
        let records = runner.live_records();
        assert_eq!(records, vec![record.clone()]);
        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&record, sink).unwrap();
        watch.connected(&mut adopted);
        adopted.write_input(b"\n").unwrap();
        wait_until("the child to finish its output", || {
            fixture.log(&record).contains(HEAVY_DONE_LINE)
        });

        let log = fs::read(record.log()).unwrap();
        let lines = String::from_utf8_lossy(&log)
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .filter(|line| line.starts_with("line-"))
            .collect::<Vec<_>>();
        let expected = (0..HEAVY_LINES)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>();
        assert!(lines == expected, "the log has a gap or a duplicate");

        let mut replayed = TerminalEmulator::new(
            Rows::try_new(ROWS).unwrap(),
            Cols::try_new(COLS).unwrap(),
            TEST_SCROLLBACK,
            ColorScheme::Dark,
        );
        replayed.process(&log);
        let holder_screen = replayed.screen_text();
        assert_eq!(screen_text(record.socket()).unwrap(), holder_screen);
        watch.until("the adopted screen to match the holder's", |watch| {
            watch.screen.screen_text() == holder_screen
        });

        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit.is_some());
    }

    #[test]
    fn a_request_the_holder_would_refuse_fails_before_anything_launches() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let missing_dir = test_request(
            "dev server",
            "true",
            Some(fixture.scratch.path().to_path_buf()),
            Some(fixture.scratch.path().join("absent")),
        );
        let no_project = test_request("dev server", "true", None, None);

        let (sink, _watch) = Watch::new();
        assert!(matches!(
            runner.spawn(missing_dir, sink),
            Err(PtyError::InvalidWorkingDir(_))
        ));
        let (sink, _watch) = Watch::new();
        assert!(runner.spawn(no_project, sink).is_err());
        assert!(
            fixture.holders.lock().unwrap().is_empty(),
            "nothing launched"
        );
    }

    /// Saves a record whose holder pid is the test process's own, alive but
    /// not a holder, as a reused pid would be, for a child running as
    /// `child_pid` that started with `child_start_token`.
    fn save_reused_pid_record(
        fixture: &Fixture,
        child_pid: Pid,
        child_start_token: Option<StartToken>,
    ) -> HolderRecord {
        let id = HolderId::generate().unwrap();
        let record = HolderRecord::builder()
            .socket(fixture.dirs.socket_path(&id))
            .log(fixture.dirs.process_log(
                &fixture.root(),
                &ProcessName::try_new("dev server").unwrap(),
            ))
            .id(id)
            .project(fixture.root())
            .process(ProcessName::try_new("dev server").unwrap())
            .command(CommandLine::try_new("true").unwrap())
            .stop(test_stop())
            .holder_pid(Pid::try_new(process::id()).unwrap())
            .child_pid(child_pid)
            .child_start_token(child_start_token)
            .child_started_at(SystemTime::now())
            .build();
        record.save(fixture.dirs.records()).unwrap();
        record
    }

    /// The test process's own pid.
    fn own_pid() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// The test process's own start token.
    fn own_start_token() -> Option<StartToken> {
        LocalProcessIdentity::start_token(own_pid())
    }

    /// A start token the test process does not have.
    fn foreign_start_token() -> Option<StartToken> {
        own_start_token().and_then(|token| StartToken::try_new(token.into_inner() + 1).ok())
    }

    #[test]
    fn stale_record_with_reused_pid_is_removed_not_adopted() {
        let fixture = Fixture::new();
        let record = save_reused_pid_record(&fixture, own_pid(), foreign_start_token());
        fixture.dirs.prepare_sockets().unwrap();
        // A datagram socket refuses every stream connect even when a child
        // another test forks inherits it; a dropped stream listener would
        // keep accepting in such a child until it execs.
        let _refusing = UnixDatagram::bind(record.socket()).unwrap();

        assert!(fixture.runner().live_records().is_empty());
        assert!(
            fixture.record_files().is_empty(),
            "a record whose socket refuses is deleted"
        );
    }

    #[test]
    fn a_record_whose_child_is_gone_and_socket_absent_is_removed() {
        let fixture = Fixture::new();
        let mut exited = process::Command::new("true").spawn().unwrap();
        let dead_pid = Pid::try_new(exited.id()).unwrap();
        exited.wait().unwrap();
        save_reused_pid_record(&fixture, dead_pid, None);

        assert!(fixture.runner().live_records().is_empty());
        assert!(
            fixture.record_files().is_empty(),
            "a record with no socket is deleted"
        );
    }

    /// A dead holder's record whose child, the same process still, runs on
    /// is kept and located, so the child is adopted as unsupervised rather
    /// than started twice; a live holder's record for the same pid wins.
    #[test]
    fn a_dead_holders_record_is_kept_while_its_child_runs_and_a_live_one_wins() {
        let fixture = Fixture::new();
        let orphaned = save_reused_pid_record(&fixture, own_pid(), own_start_token());

        assert_eq!(fixture.runner().live_records(), vec![orphaned.clone()]);
        assert_eq!(fixture.runner().locate(own_pid()), Some(orphaned));
        forget_records(&fixture);

        let fixture = Fixture::new();
        fixture.dirs.prepare_sockets().unwrap();
        let live = save_reused_pid_record(&fixture, own_pid(), own_start_token());
        let _serving = UnixListener::bind(live.socket()).unwrap();
        for _ in 0..STALE_RECORDS {
            save_reused_pid_record(&fixture, own_pid(), own_start_token());
        }
        assert_eq!(fixture.runner().locate(own_pid()), Some(live));
        forget_records(&fixture);
    }

    /// A holder too busy to accept, its backlog full, is probed in bounded
    /// time and counted as running, so a probe never stalls the service and
    /// a busy holder's record is never deleted.
    #[test]
    fn a_holder_too_busy_to_accept_is_probed_in_bounded_time_and_kept_live() {
        let fixture = Fixture::new();
        fixture.dirs.prepare_sockets().unwrap();
        let record = save_reused_pid_record(&fixture, own_pid(), foreign_start_token());
        let listener = UnixListener::bind(record.socket()).unwrap();
        listen(&listener, 0).unwrap();
        let _queued = UnixStream::connect(record.socket()).unwrap();

        let (sender, probed) = bounded(1);
        let probe = record.clone();
        thread::spawn(move || {
            let _ = sender.send(is_live(&probe));
        });

        assert_eq!(probed.recv_timeout(WAIT), Ok(true));
        assert_eq!(fixture.runner().live_records(), vec![record]);
        forget_records(&fixture);
    }

    /// Deletes the fixture's stand-in records, which name no holder, so its
    /// drop does not wait out an abandon of each.
    fn forget_records(fixture: &Fixture) {
        fs::remove_dir_all(fixture.dirs.records()).unwrap();
    }

    /// A settled socket no live holder's record names is swept, even one
    /// something still accepts on, and whatever a connection to it would
    /// say; one a live holder's record names is kept, and so is one too young
    /// to have its record yet.
    #[test]
    fn the_sweep_removes_orphan_sockets_by_record_and_age() {
        let fixture = Fixture::new();
        fixture.dirs.prepare_sockets().unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        let orphan = fixture.dirs.socket_path(&HolderId::generate().unwrap());
        drop(UnixListener::bind(&orphan).unwrap());
        let accepting = fixture.dirs.socket_path(&HolderId::generate().unwrap());
        let _accepting = UnixListener::bind(&accepting).unwrap();
        let served = save_reused_pid_record(&fixture, own_pid(), own_start_token());
        let _serving = UnixListener::bind(served.socket()).unwrap();
        let dead_holder = save_dead_holder_record(&fixture);
        drop(UnixListener::bind(dead_holder.socket()).unwrap());
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);
        let young = fixture.dirs.socket_path(&HolderId::generate().unwrap());
        drop(UnixListener::bind(&young).unwrap());

        assert!(runner.sweep_orphan_sockets().unwrap().is_empty());

        assert!(!orphan.exists(), "an unnamed socket is removed");
        assert!(!accepting.exists(), "accepting is no proof of a holder");
        assert!(
            !dead_holder.socket().exists(),
            "a dead holder's socket goes"
        );
        assert!(served.socket().exists(), "a live holder's socket is kept");
        assert!(
            young.exists(),
            "a socket younger than a holder's start stays"
        );
        forget_records(&fixture);
    }

    /// Saves a record whose holder process has exited.
    fn save_dead_holder_record(fixture: &Fixture) -> HolderRecord {
        let mut exited = process::Command::new("true").spawn().unwrap();
        let dead_pid = Pid::try_new(exited.id()).unwrap();
        exited.wait().unwrap();
        let id = HolderId::generate().unwrap();
        let name = ProcessName::try_new("dev server").unwrap();
        let record = HolderRecord::builder()
            .socket(fixture.dirs.socket_path(&id))
            .log(fixture.dirs.process_log(&fixture.root(), &name))
            .id(id)
            .project(fixture.root())
            .process(name)
            .command(CommandLine::try_new("true").unwrap())
            .stop(test_stop())
            .holder_pid(dead_pid)
            .child_pid(dead_pid)
            .child_start_token(None)
            .child_started_at(SystemTime::now())
            .build();
        record.save(fixture.dirs.records()).unwrap();
        record
    }

    #[test]
    fn a_holder_the_runner_gave_up_on_kills_its_child_and_leaves_no_record() {
        let fixture = Fixture::new();
        let runner = fixture
            .runner_with(Box::new(ThreadHolderLauncher {
                holders: Arc::clone(&fixture.holders),
                delay: SLOW_START,
            }))
            .with_ready_timeout(SHORT_READY);
        let pid_file = fixture.scratch.path().join(PID_FILE);
        let command = format!(
            "echo $$ > {}; while :; do sleep 0.05; done",
            pid_file.display()
        );

        let (sink, _watch) = Watch::new();
        assert!(runner.spawn(fixture.request(&command), sink).is_err());
        wait_until("the abandoned holder to stop", || {
            fixture
                .holders
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty(), "no record survives");
        // The kill can land before the child writes its pid; a holder that
        // finished has then already seen it exit. An absent pid file must
        // stay absent, or a surviving child wrote it late.
        let reported_pid = || {
            fs::read_to_string(&pid_file)
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
        };
        let pid = reported_pid().or_else(|| {
            thread::sleep(PID_FILE_GRACE);
            reported_pid()
        });
        if let Some(pid) = pid {
            let child = Pid::try_new(pid).unwrap();
            wait_until("the abandoned child to die", || {
                !SignalTarget::Process(child).is_alive()
            });
        }
    }

    #[test]
    fn abandoning_a_holder_kills_its_child_and_lets_it_finish() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("while :; do sleep 0.05; done"), sink)
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        drop(handle);

        abandon(record.socket());
        wait_until("the holder to finish", || {
            fixture
                .holders
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
        wait_until("the child to die", || {
            !SignalTarget::Process(record.child_pid()).is_alive()
        });
    }

    /// Abandoning a holder's handle through the port stops its child and lets
    /// the holder finish, without the caller waiting on either.
    #[test]
    fn abandoning_a_holder_handle_stops_its_child_and_lets_it_finish() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("while :; do sleep 0.05; done"), sink)
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        assert_eq!(
            handle.screen_source(),
            ScreenSource::Served(record.socket().clone())
        );

        handle.abandon();
        wait_until("the holder to finish", || {
            fixture
                .holders
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
        wait_until("the child to die", || {
            !SignalTarget::Process(record.child_pid()).is_alive()
        });
    }

    #[test]
    fn a_holder_that_fails_before_serving_reports_why() {
        let fixture = Fixture::new();
        let runner = fixture.runner_with(fixture.spec_launcher(Some(Sabotage::CorruptSpec)));

        let error = expect_failed_start(&fixture, &runner, fixture.request("true"));
        assert!(error.contains("json"), "the holder's own error: {error}");
    }

    #[test]
    fn a_holder_whose_log_cannot_open_reports_why_without_serving() {
        let fixture = Fixture::new();
        let blocker = fixture.scratch.path().join(NOT_A_DIRECTORY);
        fs::write(&blocker, b"").unwrap();
        let dirs = HolderDirs::builder()
            .sockets(fixture.dirs.sockets().clone())
            .records(fixture.dirs.records().clone())
            .logs(blocker.join("logs"))
            .build();
        let runner = fixture.runner_over(dirs, fixture.spec_launcher(None));

        let error = expect_failed_start(&fixture, &runner, fixture.request("true"));
        assert!(error.contains("i/o"), "the holder's own error: {error}");
    }

    #[test]
    fn a_holder_whose_child_cannot_spawn_reports_why_without_serving() {
        let fixture = Fixture::new();
        let workdir = fixture.scratch.path().join("work");
        fs::create_dir(&workdir).unwrap();
        let runner =
            fixture.runner_with(fixture.spec_launcher(Some(Sabotage::RemoveDir(workdir.clone()))));
        let request = test_request(
            "dev server",
            "true",
            Some(fixture.scratch.path().to_path_buf()),
            Some(workdir),
        );

        let error = expect_failed_start(&fixture, &runner, request);
        assert!(
            error.contains("working directory"),
            "the holder's own error: {error}"
        );
    }

    /// Spawns `request`, which its holder must fail to start, and returns the
    /// error, checking it carries the holder's report, arrives before the
    /// ready timeout and leaves no handover files or record behind.
    fn expect_failed_start(
        fixture: &Fixture,
        runner: &HolderRunner,
        request: SpawnRequest,
    ) -> String {
        let started = Instant::now();
        let (sink, _watch) = Watch::new();
        let error = match runner.spawn(request, sink) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a sabotaged holder cannot serve"),
        };
        assert!(error.contains("failed to start"), "error: {error}");
        assert!(
            started.elapsed() < HOLDER_READY_TIMEOUT.into_inner(),
            "the report ends the wait"
        );
        let leftovers = fs::read_dir(fixture.dirs.sockets()).unwrap().count();
        assert_eq!(leftovers, 0, "no socket, spec or error file is left");
        assert!(fixture.record_files().is_empty(), "no record is left");
        error
    }

    #[test]
    fn abandoning_a_holder_still_starting_reaches_it_once_it_serves() {
        let fixture = Fixture::new();
        fixture.dirs.prepare_sockets().unwrap();
        let config = fixture.config("while :; do sleep 0.05; done", LONG_SUPERVISOR_DEADLINE);
        let socket = config.socket().clone();
        ThreadHolderLauncher {
            holders: Arc::clone(&fixture.holders),
            delay: STARTING_DELAY,
        }
        .launch(config)
        .unwrap();

        abandon(&socket);
        wait_until("the abandoned holder to finish", || {
            fixture
                .holders
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
    }

    #[test]
    fn terminate_honours_signal_and_grace_then_kills() {
        let fixture = Fixture::new();
        let runner = fixture.runner();

        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request(
                    "trap 'printf got-int; exit 0' INT; printf ready; while :; do sleep 0.05; done",
                ),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the trap to be set", |watch| watch.text().contains("ready"));
        let started = Instant::now();
        handle.terminate(StopSignal::Interrupt, WAIT).unwrap();
        watch.until("the graceful exit", |watch| watch.exit.is_some());
        assert!(watch.text().contains("got-int"), "the signal was delivered");
        assert_eq!(watch.exit, Some(ExitOutcome::Succeeded));
        assert!(started.elapsed() < WAIT, "a graceful exit needs no kill");

        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request("trap '' TERM; printf ready; while :; do sleep 0.05; done"),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the trap to be set", |watch| watch.text().contains("ready"));
        let started = Instant::now();
        handle.terminate(StopSignal::Terminate, STOP_GRACE).unwrap();
        thread::sleep(STOP_GRACE / 2);
        watch.drain();
        assert!(
            watch.exit.is_none(),
            "the child outlives the ignored signal within its grace"
        );
        watch.until("the forced exit", |watch| watch.exit.is_some());
        assert!(
            matches!(watch.exit, Some(ExitOutcome::Failed(ExitCause::Signal(_)))),
            "the forced stop ends the child by a signal: {:?}",
            watch.exit
        );
        assert!(
            started.elapsed() >= STOP_GRACE,
            "the kill waits out the grace"
        );
    }

    /// Sweeping removes the spec and error files beside holder sockets and
    /// nothing else; a missing socket directory holds nothing to sweep.
    #[test]
    fn sweeping_removes_only_handover_files() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        assert!(runner.sweep_handover_files().unwrap().is_empty());
        fixture.dirs.prepare_sockets().unwrap();
        let id = HolderId::generate().unwrap();
        let socket = fixture.dirs.socket_path(&id);
        let spec = fixture.config("true", LONG_SUPERVISOR_DEADLINE).spec_path();
        let error = error_path_beside(&spec);
        for path in [&socket, &spec, &error] {
            fs::write(path, b"x").unwrap();
        }
        let stuck = fixture.dirs.sockets().join(STUCK_HANDOVER);
        fs::create_dir(&stuck).unwrap();

        let failures = runner.sweep_handover_files().unwrap();

        assert!(socket.exists());
        assert!(
            !spec.exists() && !error.exists(),
            "the sweep goes on past a failure"
        );
        assert_eq!(
            failures
                .into_iter()
                .map(|(path, _)| path)
                .collect::<Vec<_>>(),
            vec![stuck]
        );
    }

    /// A record file that cannot be parsed is reported, never adopted.
    #[test]
    fn an_unreadable_record_is_reported() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.dirs.records()).unwrap();
        let corrupt = fixture.dirs.records().join("corrupt.json");
        fs::write(&corrupt, CORRUPT_SPEC).unwrap();

        let runner = fixture.runner();

        assert_eq!(runner.unreadable_records(), vec![corrupt]);
        assert!(runner.live_records().is_empty());
    }
}
