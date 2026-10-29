//! Test doubles for the ports a supervisor drives, shared by its tests.

use std::{
    cell::RefCell,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::PathBuf,
    process::{Child, Command},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use bytesize::ByteSize as RawByteSize;
use crossbeam_channel::{Receiver, unbounded};
use rustix::process::Signal;

use super::{SpawnGeneration, Supervisor, SupervisorDeps};
use crate::{
    adapter::{
        service::{
            EventLog, LaunchEnvironment, Notices, ServiceEvent, SharedEventLog, SharedNotices,
        },
        signal_target::SignalTarget,
    },
    application::Workspace,
    domain::{
        color_scheme::ColorScheme,
        config::{Environment, McpStackEdits, ProcessSpec, Stack},
        notification::Notification,
        port::{Notifier, OutputSink, ProcessHandle, ProcessInput, ProcessRunner},
        process::{Process, RestartPolicy, StopPolicy, StopSignal},
        project::Project,
        pty::{PtyError, PtySize, ScreenSource, SpawnRequest},
        settings::Settings,
        value::{
            ByteSize, CommandLine, DetachKey, LogMaxSize, PaneId, Pid, ProcessName, ProjectName,
            ProjectRoot, ScrollbackLines, StopGrace,
        },
    },
};

/// Folder of the project every harness supervisor runs.
pub(crate) const TEST_PROJECT: &str = "/here";
/// Terminal rows and columns every harness supervisor starts processes with.
const TEST_TERMINAL_SIDE: u16 = 24;
/// Scrollback lines every harness launch asks for.
const TEST_SCROLLBACK: usize = 100;
/// Log cap of every harness launch, in MiB.
const TEST_LOG_CAP_MIB: u64 = 4;
/// Grace of every harness process's stop policy.
pub(crate) const TEST_STOP_GRACE: Duration = Duration::from_secs(7);

/// The process name `raw`.
pub(crate) fn name(raw: &str) -> ProcessName {
    ProcessName::try_new(raw).unwrap()
}

/// The project every harness supervisor runs.
pub(crate) fn test_project() -> Project {
    let root = ProjectRoot::try_new(PathBuf::from(TEST_PROJECT)).unwrap();
    let name = ProjectName::candidates(&root).next().unwrap();
    Project::builder().name(name).root(root).build()
}

/// The stop policy every harness process has: an interrupt, then a kill
/// after [`TEST_STOP_GRACE`].
pub(crate) fn test_stop() -> StopPolicy {
    StopPolicy::builder()
        .signal(StopSignal::Interrupt)
        .grace_period(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
        .build()
}

/// An autostarting process `raw` running `command` under `restart`.
pub(crate) fn spec(raw: &str, command: &str, restart: RestartPolicy) -> ProcessSpec {
    ProcessSpec::builder()
        .name(name(raw))
        .command(CommandLine::try_new(command).unwrap())
        .environment(Environment::empty())
        .autostart(true)
        .restart(restart)
        .stop(test_stop())
        .build()
}

/// The harness project's stack of `processes`.
pub(crate) fn stack(processes: &[ProcessSpec]) -> Stack {
    Stack::builder()
        .project(test_project())
        .processes(processes.to_vec())
        .build()
}

/// Every setting explicit, desktop notifications off.
pub(crate) fn test_settings() -> Settings {
    Settings::builder()
        .desktop_notifications(false)
        .default_size(PtySize::try_new(TEST_TERMINAL_SIDE, TEST_TERMINAL_SIDE).unwrap())
        .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
        .log_max_size(
            LogMaxSize::try_new(ByteSize::try_new(RawByteSize::mib(TEST_LOG_CAP_MIB)).unwrap())
                .unwrap(),
        )
        .scrollback(ScrollbackLines::try_new(TEST_SCROLLBACK).unwrap())
        .color_scheme(ColorScheme::Dark)
        .default_autostart(false)
        .default_restart(RestartPolicy::Never)
        .default_stop_signal(StopSignal::Terminate)
        .default_stop_grace(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
        .mcp_stack_edits(McpStackEdits::Allow)
        .build()
}

/// A supervisor under test, with the channel its timers and output arrive on
/// and the notices it raises.
pub(crate) struct Harness {
    pub(crate) supervisor: Supervisor,
    pub(crate) events: Receiver<ServiceEvent>,
    pub(crate) notices: SharedNotices,
    pub(crate) event_log: SharedEventLog,
}

impl Harness {
    /// A supervisor of [`TEST_PROJECT`] running a stack of `processes`,
    /// spawning through `runner`, with nothing started yet.
    pub(crate) fn over(runner: Box<dyn ProcessRunner>, processes: &[ProcessSpec]) -> Self {
        let (sender, events) = unbounded();
        let notices = Notices::shared(None);
        let event_log = EventLog::shared();
        let deps = SupervisorDeps::builder()
            .runner(runner)
            .events(sender)
            .notices(notices.clone())
            .event_log(event_log.clone())
            .probes(None)
            .notifier(None)
            .port_occupants(None)
            .settings(test_settings())
            .build();
        Self {
            supervisor: Supervisor::new(test_project(), &stack(processes), deps),
            events,
            notices,
            event_log,
        }
    }

    /// A supervisor of [`TEST_PROJECT`] whose workspace is exactly
    /// `processes`, panes and states as given, spawning through `runner`.
    pub(crate) fn new(runner: Box<dyn ProcessRunner>, processes: Vec<Process>) -> Self {
        let mut harness = Self::over(runner, &[]);
        *harness.supervisor.workspace_mut() = Workspace::builder().processes(processes).build();
        harness
    }

    /// Brings the project up as a client with no environment of its own would.
    pub(crate) fn up(&mut self) {
        self.supervisor.up(&LaunchEnvironment::default());
    }

    /// The messages of every notice raised so far, oldest first.
    pub(crate) fn notice_messages(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap()
            .recent(self.supervisor.root())
            .iter()
            .map(|notice| notice.message().clone())
            .collect()
    }
}

/// What an [`Orphan`] runs: a long sleep it never finishes on its own.
const ORPHAN_PROGRAM: &str = "sleep";
/// The orphan's sleep, in seconds, far past any test.
const ORPHAN_SECONDS: &str = "600";
/// Longest an orphan's drop waits for it to be gone.
const ORPHAN_REAP_WAIT: Duration = Duration::from_secs(10);
/// Interval between checks that a killed orphan is gone.
const ORPHAN_POLL: Duration = Duration::from_millis(10);

/// A real process in its own process group that ignores the graceful stop
/// signals, standing in for a child left running under a holder the service
/// cannot reach. By default a reaper thread collects it, so a killed orphan is
/// gone rather than a zombie, and reports the signal that ended it. Dropping
/// it kills the group, also on a panic.
pub(crate) struct Orphan {
    pid: Pid,
    /// The signal that ended a reaped orphan, once it ended.
    ended_by: Option<Receiver<Option<i32>>>,
    /// An orphan nobody reaps, which stays a zombie once killed until dropped.
    unreaped: Option<Child>,
}

impl Orphan {
    /// Starts an orphan that is reaped as soon as it dies.
    pub(crate) fn spawn() -> Self {
        let mut child = Self::command().spawn().unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let (sender, ended_by) = unbounded();
        thread::spawn(move || {
            let signal = child.wait().ok().and_then(|status| status.signal());
            let _ = sender.send(signal);
        });
        Self {
            pid,
            ended_by: Some(ended_by),
            unreaped: None,
        }
    }

    /// Starts an orphan nobody reaps, so once killed it lingers as a zombie
    /// whose pid still exists.
    pub(crate) fn unreaped() -> Self {
        let child = Self::command().spawn().unwrap();
        Self {
            pid: Pid::try_new(child.id()).unwrap(),
            ended_by: None,
            unreaped: Some(child),
        }
    }

    /// The orphan's command: a long sleep in its own process group, ignoring
    /// SIGINT and SIGTERM.
    fn command() -> Command {
        let mut command = Command::new(ORPHAN_PROGRAM);
        command.arg(ORPHAN_SECONDS).process_group(0);
        // SAFETY: the hook only resets two signal dispositions, which is
        // async-signal-safe between fork and exec.
        unsafe {
            command.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_IGN);
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
                Ok(())
            });
        }
        command
    }

    /// The orphan's process id, which also names its process group.
    pub(crate) fn pid(&self) -> Pid {
        self.pid
    }

    /// Whether the orphan still exists.
    pub(crate) fn alive(&self) -> bool {
        SignalTarget::Process(self.pid).is_alive()
    }

    /// Sends `signal` to a reaped orphan and returns the signal that ended
    /// it, waiting at most [`ORPHAN_REAP_WAIT`]. A signal something else sent
    /// first shows up as that signal instead.
    pub(crate) fn end_with(&self, signal: i32) -> Option<i32> {
        if let Some(signal) = Signal::from_named_raw(signal) {
            let _ = SignalTarget::Process(self.pid).send(signal);
        }
        self.ended_by
            .as_ref()?
            .recv_timeout(ORPHAN_REAP_WAIT)
            .ok()
            .flatten()
    }
}

impl Drop for Orphan {
    /// Kills the orphan's group and waits, bounded, until it is gone.
    fn drop(&mut self) {
        let _ = SignalTarget::Group(self.pid).send(Signal::KILL);
        if let Some(mut child) = self.unreaped.take() {
            let _ = child.wait();
            return;
        }
        let started = Instant::now();
        while self.alive() && started.elapsed() < ORPHAN_REAP_WAIT {
            thread::sleep(ORPHAN_POLL);
        }
    }
}

/// A notifier that records every notification delivered.
#[derive(Clone, Default)]
pub(crate) struct RecordingNotifier {
    delivered: Rc<RefCell<Vec<Notification>>>,
}

impl RecordingNotifier {
    /// Every notification delivered, oldest first.
    pub(crate) fn delivered(&self) -> Vec<Notification> {
        self.delivered.borrow().clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, notification: &Notification) {
        self.delivered.borrow_mut().push(notification.clone());
    }
}

/// A runner whose spawned processes never emit output and never exit on their
/// own; the test drives exits explicitly.
pub(crate) struct FakeRunner;

impl ProcessRunner for FakeRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(FakeHandle))
    }
}

/// Shared record of complete PTY requests for command and environment
/// assertions.
#[derive(Clone, Default)]
pub(crate) struct SpawnRecorder {
    pub(crate) requests: Rc<RefCell<Vec<SpawnRequest>>>,
}

/// A successful runner that retains each spawn request.
pub(crate) struct RequestRecordingRunner {
    pub(crate) recorder: SpawnRecorder,
}

impl ProcessRunner for RequestRecordingRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        self.recorder.requests.borrow_mut().push(request);
        Ok(Box::new(FakeHandle))
    }
}

/// The pid a [`PidHandle`] reports.
const FAKE_LAUNCHER_PID: u32 = 4242;

/// The pid a [`PidHandle`] reports, as a [`Pid`].
pub(crate) fn fake_launcher_pid() -> Pid {
    Pid::try_new(FAKE_LAUNCHER_PID).unwrap()
}

/// A handle that reports a pid, like the real PTY handle does.
pub(crate) struct PidHandle;

impl ProcessHandle for PidHandle {
    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn process_id(&self) -> Option<Pid> {
        Some(fake_launcher_pid())
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}

/// A successful runner whose handles report a pid.
pub(crate) struct PidRunner;

impl ProcessRunner for PidRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(PidHandle))
    }
}

/// A handle every operation of which succeeds and does nothing.
pub(crate) struct FakeHandle;

impl ProcessHandle for FakeHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}

/// A runner that succeeds once, then fails every subsequent spawn.
pub(crate) struct FlakyRunner {
    pub(crate) spawns: AtomicUsize,
}

impl ProcessRunner for FlakyRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if self.spawns.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Box::new(FakeHandle))
        } else {
            Err(PtyError::System("spawn failed".to_string()))
        }
    }
}

/// A runner whose every spawn fails, modelling a persistent PTY or resource
/// failure.
pub(crate) struct FailingRunner;

impl ProcessRunner for FailingRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Err(PtyError::System("spawn failed".to_string()))
    }
}

/// A handle that records everything written to its PTY.
pub(crate) struct RecordingHandle {
    pub(crate) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessHandle for RecordingHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.written.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}

/// A runner whose spawned processes record their input.
pub(crate) struct RecordingRunner {
    pub(crate) written: Arc<Mutex<Vec<u8>>>,
}

impl ProcessRunner for RecordingRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(RecordingHandle {
            written: self.written.clone(),
        }))
    }
}

/// A runner whose handles count how many times they are killed.
pub(crate) struct KillCountRunner {
    pub(crate) kills: Arc<AtomicUsize>,
}

impl ProcessRunner for KillCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(KillCountHandle {
            kills: self.kills.clone(),
        }))
    }
}

/// A handle that counts how many times it is killed.
pub(crate) struct KillCountHandle {
    pub(crate) kills: Arc<AtomicUsize>,
}

impl ProcessHandle for KillCountHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The screen a [`ConnectedHandle`] connects to.
pub(crate) const FIRST_SCREEN: &[u8] = b"first ";
/// Longest a test waits for a worker's launch result.
const WORKER_WAIT: Duration = Duration::from_secs(5);

/// A handle that connected to a screen already showing [`FIRST_SCREEN`],
/// counting the kills it is sent.
pub(crate) struct ConnectedHandle(Arc<AtomicUsize>);

impl ProcessHandle for ConnectedHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        Some(FIRST_SCREEN.to_vec())
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A runner whose launches run on a worker, as a holder runner's do; the
/// shared count is the kills its handles were sent.
#[derive(Clone, Default)]
pub(crate) struct WorkerRunner(pub(crate) Arc<AtomicUsize>);

impl ProcessRunner for WorkerRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(ConnectedHandle(Arc::clone(&self.0))))
    }

    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}

/// The launch result a worker posted for `harness`, which the test holds
/// until it chooses to settle it, as a gated launch would.
pub(crate) fn spawned(
    harness: &Harness,
) -> (
    PaneId,
    SpawnGeneration,
    Result<Box<dyn ProcessHandle>, PtyError>,
) {
    match harness.events.recv_timeout(WORKER_WAIT) {
        Ok(ServiceEvent::Spawned {
            pane,
            generation,
            result,
            ..
        }) => (pane, generation, result),
        _ => panic!("the worker posts the launch result"),
    }
}

/// Counts graceful termination requests separately from hard kills.
#[derive(Default)]
pub(crate) struct StopSignals {
    pub(crate) terminates: AtomicUsize,
    pub(crate) kills: AtomicUsize,
    pub(crate) termination: Mutex<Option<(StopSignal, Duration)>>,
}

/// A runner whose handles record which stop mechanism was selected.
pub(crate) struct StopSignalRunner {
    pub(crate) signals: Arc<StopSignals>,
}

impl ProcessRunner for StopSignalRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(StopSignalHandle {
            signals: self.signals.clone(),
        }))
    }
}

/// A handle that distinguishes graceful termination from a hard kill.
pub(crate) struct StopSignalHandle {
    pub(crate) signals: Arc<StopSignals>,
}

impl ProcessHandle for StopSignalHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates.fetch_add(1, Ordering::SeqCst);
        *self.signals.termination.lock().unwrap() = Some((signal, grace));
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.signals.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Counts stop attempts for a handle whose first hard kill fails.
#[derive(Default)]
pub(crate) struct RetryStopSignals {
    pub(crate) terminates: AtomicUsize,
    pub(crate) kills: AtomicUsize,
}

/// Configured result of graceful termination in retry tests.
#[derive(Clone, Copy)]
pub(crate) enum TerminateOutcome {
    Succeeds,
    Fails,
}

/// A runner used to verify that failed stop delivery remains retryable.
pub(crate) struct RetryStopRunner {
    pub(crate) signals: Arc<RetryStopSignals>,
    pub(crate) terminate_outcome: TerminateOutcome,
}

impl ProcessRunner for RetryStopRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(RetryStopHandle {
            signals: self.signals.clone(),
            terminate_outcome: self.terminate_outcome,
        }))
    }
}

/// A handle whose graceful termination follows its configured outcome and
/// whose hard kill succeeds on the second attempt.
pub(crate) struct RetryStopHandle {
    pub(crate) signals: Arc<RetryStopSignals>,
    pub(crate) terminate_outcome: TerminateOutcome,
}

impl ProcessHandle for RetryStopHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates.fetch_add(1, Ordering::SeqCst);
        match self.terminate_outcome {
            TerminateOutcome::Succeeds => Ok(()),
            TerminateOutcome::Fails => Err(PtyError::Unsupported("signal failed".to_string())),
        }
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        let attempt = self.signals.kills.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(PtyError::Unsupported("signal failed".to_string()))
        } else {
            Ok(())
        }
    }
}

/// A runner whose handles fail every pause and resume, modelling a signal that
/// cannot be delivered.
pub(crate) struct PauseFailRunner;

impl ProcessRunner for PauseFailRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(PauseFailHandle))
    }
}

/// A handle whose pause and resume always fail.
pub(crate) struct PauseFailHandle;

impl ProcessHandle for PauseFailHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported("signal failed".to_string()))
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Err(PtyError::Unsupported("signal failed".to_string()))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}

/// A runner whose handles count the exits acknowledged to them.
pub(crate) struct AckCountRunner {
    pub(crate) acks: Arc<AtomicUsize>,
}

impl ProcessRunner for AckCountRunner {
    fn spawn(
        &self,
        _request: SpawnRequest,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(AckCountHandle {
            acks: Arc::clone(&self.acks),
        }))
    }
}

/// A handle that counts the exits acknowledged to it.
pub(crate) struct AckCountHandle {
    pub(crate) acks: Arc<AtomicUsize>,
}

impl ProcessHandle for AckCountHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Mirror
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        self.acks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}
