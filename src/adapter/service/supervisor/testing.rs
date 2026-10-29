//! Test doubles for the ports a supervisor drives, shared by its tests.

use std::{
    cell::RefCell,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, unbounded};

use super::{SpawnGeneration, Supervisor, SupervisorDeps};
use crate::{
    adapter::service::{Notices, ProjectKey, ServiceEvent, SharedNotices},
    application::Workspace,
    domain::{
        agent_session::{
            AgentProcessId, AgentProcessStartToken, AgentSession, AgentSessionId,
            AgentSessionState, ConfiguredAgentKey, LaunchToken, NativeSessionId,
        },
        color_scheme::ColorScheme,
        config::{ConfigError, WorkspaceConfig},
        notification::{Notification, NotificationId, NotificationScope},
        port::{
            AgentSessionStore, Notifier, OutputSink, ProcessHandle, ProcessRunner, ProjectRegistry,
        },
        process::{AgentTool, Process, StopSignal},
        project::Project,
        pty::{PtyError, PtySize, SpawnRequest},
        value::{Cols, CommandLine, PaneId, ProcessName, Rows},
    },
};

/// Config path every harness supervisor runs.
pub(crate) const TEST_PROJECT: &str = "/here/muster.yml";
/// Terminal rows and columns every harness supervisor starts processes with.
const TEST_TERMINAL_SIDE: u16 = 24;

/// A supervisor under test, with the channel its timers and output arrive on
/// and the notices it raises.
pub(crate) struct Harness {
    pub(crate) supervisor: Supervisor,
    pub(crate) events: Receiver<ServiceEvent>,
    pub(crate) notices: SharedNotices,
}

impl Harness {
    /// A supervisor of [`TEST_PROJECT`] over `processes`, spawning through
    /// `runner`, with no agent-session store and nothing started yet.
    pub(crate) fn new(runner: Box<dyn ProcessRunner>, processes: Vec<Process>) -> Self {
        Self::with_registry(runner, processes, empty_registry())
    }

    /// As [`Self::new`], reading and writing the project's config through
    /// `registry`.
    pub(crate) fn with_registry(
        runner: Box<dyn ProcessRunner>,
        processes: Vec<Process>,
        registry: Box<dyn ProjectRegistry>,
    ) -> Self {
        let (sender, events) = unbounded();
        let notices = Notices::shared(None);
        let deps = SupervisorDeps::builder()
            .runner(runner)
            .registry(registry)
            .agent_session_store(None)
            .events(sender)
            .notices(notices.clone())
            .config_watcher(None)
            .notifier(None)
            .desktop_notifications(false)
            .process_metrics(None)
            .size(
                PtySize::builder()
                    .rows(Rows::new(TEST_TERMINAL_SIDE))
                    .cols(Cols::new(TEST_TERMINAL_SIDE))
                    .build(),
            )
            .color_scheme(ColorScheme::Dark)
            .build();
        let supervisor = Supervisor::new(
            ProjectKey::new(PathBuf::from(TEST_PROJECT)),
            Workspace::builder().processes(processes).build(),
            deps,
        );
        Self {
            supervisor,
            events,
            notices,
        }
    }

    /// The messages of every notice raised so far, oldest first.
    pub(crate) fn notice_messages(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap()
            .recent(self.supervisor.project())
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
    pid: u32,
    /// The signal that ended a reaped orphan, once it ended.
    ended_by: Option<Receiver<Option<i32>>>,
    /// An orphan nobody reaps, which stays a zombie once killed until dropped.
    unreaped: Option<Child>,
}

impl Orphan {
    /// Starts an orphan that is reaped as soon as it dies.
    pub(crate) fn spawn() -> Self {
        let mut child = Self::command().spawn().unwrap();
        let pid = child.id();
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
            pid: child.id(),
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
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the orphan still exists.
    pub(crate) fn alive(&self) -> bool {
        // SAFETY: signal 0 only checks that the pid exists.
        unsafe { libc::kill(self.pid as libc::pid_t, 0) == 0 }
    }

    /// Sends `signal` to a reaped orphan and returns the signal that ended
    /// it, waiting at most [`ORPHAN_REAP_WAIT`]. A signal something else sent
    /// first shows up as that signal instead.
    pub(crate) fn end_with(&self, signal: i32) -> Option<i32> {
        // SAFETY: `kill` takes plain integers; the pid is the orphan's own.
        unsafe {
            libc::kill(self.pid as libc::pid_t, signal);
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
        // SAFETY: `kill` takes plain integers; the group is the orphan's own.
        unsafe {
            libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
        }
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

/// A notifier that records delivered and closed notifications.
#[derive(Clone, Default)]
pub(crate) struct RecordingNotifier {
    delivered: Rc<RefCell<Vec<Notification>>>,
    closed: Rc<RefCell<Vec<(PaneId, NotificationId)>>>,
}

impl RecordingNotifier {
    /// Number of notifications delivered.
    pub(crate) fn count(&self) -> usize {
        self.delivered.borrow().len()
    }

    /// The pane each delivered notification came from.
    pub(crate) fn panes(&self) -> Vec<PaneId> {
        self.delivered
            .borrow()
            .iter()
            .map(|notification| *notification.pane())
            .collect()
    }

    /// Identifiers carried by delivered notifications.
    pub(crate) fn identifiers(&self) -> Vec<Option<NotificationId>> {
        self.delivered
            .borrow()
            .iter()
            .map(|notification| notification.identifier().clone())
            .collect()
    }

    /// Bodies carried by delivered notifications.
    pub(crate) fn bodies(&self) -> Vec<Option<String>> {
        self.delivered
            .borrow()
            .iter()
            .map(|notification| notification.body().clone())
            .collect()
    }

    /// Identifiers passed to close requests.
    pub(crate) fn closed(&self) -> Vec<(PaneId, NotificationId)> {
        self.closed.borrow().clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, notification: &Notification) {
        self.delivered.borrow_mut().push(notification.clone());
    }

    fn close(&self, pane: PaneId, _scope: NotificationScope, identifier: &NotificationId) {
        self.closed.borrow_mut().push((pane, identifier.clone()));
    }
}

/// A shared record of what a [`FakeRegistry`] was asked to persist.
#[derive(Clone, Default)]
pub(crate) struct Recorder {
    pub(crate) projects: Rc<RefCell<Option<Vec<Project>>>>,
    pub(crate) workspaces: Rc<RefCell<Vec<(PathBuf, WorkspaceConfig)>>>,
}

/// Shared in-memory durable state for agent-session lifecycle tests.
#[derive(Clone, Default)]
pub(crate) struct SessionRecorder {
    pub(crate) sessions: Rc<RefCell<Vec<AgentSession>>>,
}

/// An agent-session store with the same history ordering semantics as the
/// YAML adapter, without touching platform state.
pub(crate) struct FakeAgentSessionStore {
    pub(crate) recorder: SessionRecorder,
}

impl AgentSessionStore for FakeAgentSessionStore {
    fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        Ok(self.recorder.sessions.borrow().clone())
    }

    fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
        Ok(None)
    }

    fn upsert(&self, session: &AgentSession) -> Result<(), ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        sessions.retain(|candidate| candidate.id() != session.id());
        sessions.push(session.clone());
        Ok(())
    }

    fn link_configured(&self, candidate: &AgentSession) -> Result<AgentSessionId, ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        if let Some(session) = sessions.iter_mut().find(|session| {
            session.project() == candidate.project()
                && session.configured_key().is_some()
                && session.configured_key() == candidate.configured_key()
        }) {
            *session = session
                .clone()
                .with_configured_command(*candidate.tool(), candidate.launch_command().clone());
            Ok(session.id().clone())
        } else {
            sessions.push(candidate.clone());
            Ok(candidate.id().clone())
        }
    }

    fn retire_orphaned_configured(
        &self,
        project: &Path,
        live_keys: &[ConfiguredAgentKey],
    ) -> Result<usize, ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        let mut retired = 0;
        for session in sessions.iter_mut() {
            let orphaned = session.project() == project
                && session
                    .configured_key()
                    .as_ref()
                    .is_some_and(|key| !live_keys.contains(key));
            if orphaned {
                *session = session
                    .clone()
                    .unconfigure()
                    .with_state(AgentSessionState::Closed);
                retired += 1;
            }
        }
        Ok(retired)
    }

    fn set_state(&self, id: &AgentSessionId, state: AgentSessionState) -> Result<(), ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        let index = sessions
            .iter()
            .position(|session| session.id() == id)
            .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
        let session = sessions.remove(index).with_state(state);
        sessions.push(session);
        Ok(())
    }

    fn set_owner_process_id(
        &self,
        id: &AgentSessionId,
        process_id: AgentProcessId,
        process_start_token: Option<AgentProcessStartToken>,
        launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        let session = sessions
            .iter_mut()
            .find(|session| session.id() == id)
            .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
        *session = session
            .clone()
            .with_launch_owner(process_id, process_start_token, launch_token);
        Ok(())
    }

    fn capture_native_id(
        &self,
        id: &AgentSessionId,
        provider: AgentTool,
        native_id: NativeSessionId,
        reporter_process_id: AgentProcessId,
        reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let mut sessions = self.recorder.sessions.borrow_mut();
        let session = sessions
            .iter_mut()
            .find(|session| session.id() == id)
            .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
        if *session.tool() != provider {
            return Err(ConfigError::AgentSessionProviderMismatch {
                id: id.clone(),
                expected: *session.tool(),
                reported: provider,
            });
        }
        if let Some(current) = session.launch_token()
            && reported_launch_token.as_ref() != Some(current)
        {
            return Ok(());
        }
        *session = session
            .clone()
            .with_reported_native_id(native_id, reporter_process_id, None);
        Ok(())
    }
}

/// A store that models an unavailable platform state directory.
pub(crate) struct FailingAgentSessionStore;

impl AgentSessionStore for FailingAgentSessionStore {
    fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        Ok(Vec::new())
    }

    fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
        Ok(None)
    }

    fn upsert(&self, _session: &AgentSession) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn link_configured(&self, _candidate: &AgentSession) -> Result<AgentSessionId, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn set_state(
        &self,
        _id: &AgentSessionId,
        _state: AgentSessionState,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn set_owner_process_id(
        &self,
        _id: &AgentSessionId,
        _process_id: AgentProcessId,
        _process_start_token: Option<AgentProcessStartToken>,
        _launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn capture_native_id(
        &self,
        _id: &AgentSessionId,
        _provider: AgentTool,
        _native_id: NativeSessionId,
        _reporter_process_id: AgentProcessId,
        _reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn retire_orphaned_configured(
        &self,
        _project: &Path,
        _live_keys: &[ConfiguredAgentKey],
    ) -> Result<usize, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }
}

/// A store that fails reads while preserving the rest of the port shape.
pub(crate) struct UnreadableAgentSessionStore;

impl AgentSessionStore for UnreadableAgentSessionStore {
    fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn link_configured(&self, _candidate: &AgentSession) -> Result<AgentSessionId, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn upsert(&self, _session: &AgentSession) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn set_state(
        &self,
        _id: &AgentSessionId,
        _state: AgentSessionState,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn set_owner_process_id(
        &self,
        _id: &AgentSessionId,
        _process_id: AgentProcessId,
        _process_start_token: Option<AgentProcessStartToken>,
        _launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn capture_native_id(
        &self,
        _id: &AgentSessionId,
        _provider: AgentTool,
        _native_id: NativeSessionId,
        _reporter_process_id: AgentProcessId,
        _reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::NoConfigDir)
    }

    fn retire_orphaned_configured(
        &self,
        _project: &Path,
        _live_keys: &[ConfiguredAgentKey],
    ) -> Result<usize, ConfigError> {
        Err(ConfigError::NoConfigDir)
    }
}

/// A registry returning a fixed project list and one workspace config, and
/// recording everything it is asked to save.
pub(crate) struct FakeRegistry {
    pub(crate) projects: Vec<Project>,
    pub(crate) workspace: WorkspaceConfig,
    pub(crate) recorder: Recorder,
}

impl ProjectRegistry for FakeRegistry {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        Ok(self
            .recorder
            .projects
            .borrow()
            .clone()
            .unwrap_or_else(|| self.projects.clone()))
    }

    fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
        Ok(self.workspace.clone())
    }

    fn workspace_exists(&self, config_path: &Path) -> bool {
        self.recorder
            .workspaces
            .borrow()
            .iter()
            .any(|(path, _)| path == config_path)
    }

    fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
        *self.recorder.projects.borrow_mut() = Some(projects.to_vec());
        Ok(())
    }

    fn save_workspace(
        &self,
        config_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<(), ConfigError> {
        self.recorder
            .workspaces
            .borrow_mut()
            .push((config_path.to_path_buf(), config.clone()));
        Ok(())
    }
}

/// A workspace config with no processes in any section.
pub(crate) fn empty_workspace_config() -> WorkspaceConfig {
    WorkspaceConfig::builder()
        .agents(vec![])
        .terminals(vec![])
        .commands(vec![])
        .build()
}

/// A registry with no registered projects.
pub(crate) fn empty_registry() -> Box<dyn ProjectRegistry> {
    Box::new(FakeRegistry {
        projects: Vec::new(),
        workspace: empty_workspace_config(),
        recorder: Recorder::default(),
    })
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

/// The pid a [`PidHandle`] reports, asserted by the claim tests.
pub(crate) const FAKE_LAUNCHER_PID: u32 = 4242;

/// A handle that reports a pid, like the real PTY handle does.
pub(crate) struct PidHandle;

impl ProcessHandle for PidHandle {
    fn process_id(&self) -> Option<u32> {
        Some(FAKE_LAUNCHER_PID)
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

/// A successful runner whose handles expose a pid.
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

/// A started supervisor of [`TEST_PROJECT`] with no processes, durable history
/// seeded with `initial` and every spawn request recorded.
pub(crate) fn agent_harness(
    initial: Vec<AgentSession>,
) -> (Harness, SessionRecorder, SpawnRecorder) {
    agent_harness_over(initial, empty_registry())
}

/// As [`agent_harness`], reading and writing the config through `registry`.
pub(crate) fn agent_harness_over(
    initial: Vec<AgentSession>,
    registry: Box<dyn ProjectRegistry>,
) -> (Harness, SessionRecorder, SpawnRecorder) {
    let sessions = SessionRecorder {
        sessions: Rc::new(RefCell::new(initial)),
    };
    let spawns = SpawnRecorder::default();
    let mut harness = Harness::with_registry(
        Box::new(RequestRecordingRunner {
            recorder: spawns.clone(),
        }),
        Vec::new(),
        registry,
    );
    harness
        .supervisor
        .set_agent_session_store(Box::new(FakeAgentSessionStore {
            recorder: sessions.clone(),
        }));
    harness.supervisor.up();
    (harness, sessions, spawns)
}

/// A durable provider session of [`TEST_PROJECT`] named `Ada`, for restore
/// and history tests.
pub(crate) fn persisted_agent_session(
    tool: AgentTool,
    native_id: &str,
    state: AgentSessionState,
) -> AgentSession {
    AgentSession::builder()
        .id(AgentSessionId::generate().unwrap())
        .name(ProcessName::try_new("Ada").unwrap())
        .tool(tool)
        .project(PathBuf::from(TEST_PROJECT))
        .launch_command(CommandLine::try_new(tool.default_command().unwrap()).unwrap())
        .native_id(Some(NativeSessionId::try_new(native_id).unwrap()))
        .state(state)
        .build()
}

/// A closed, resumable conversation (captured native id, no configured link)
/// of `project`.
pub(crate) fn closed_conversation(name: &str, project: &str, native_id: &str) -> AgentSession {
    AgentSession::builder()
        .id(AgentSessionId::generate().unwrap())
        .name(ProcessName::try_new(name).unwrap())
        .tool(AgentTool::Claude)
        .project(PathBuf::from(project))
        .launch_command(CommandLine::try_new("claude").unwrap())
        .native_id(Some(NativeSessionId::try_new(native_id).unwrap()))
        .state(AgentSessionState::Closed)
        .build()
}

/// A registry whose `workspace` reflects the latest `save_workspace`, so a
/// sequence of edits accumulates as the filesystem adapter's do.
pub(crate) struct StatefulRegistry {
    pub(crate) recorder: Recorder,
    pub(crate) initial: WorkspaceConfig,
}

impl ProjectRegistry for StatefulRegistry {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        Ok(self.recorder.projects.borrow().clone().unwrap_or_default())
    }

    fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
        Ok(self
            .recorder
            .workspaces
            .borrow()
            .last()
            .map(|(_, config)| config.clone())
            .unwrap_or_else(|| self.initial.clone()))
    }

    fn workspace_exists(&self, _config_path: &Path) -> bool {
        true
    }

    fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
        *self.recorder.projects.borrow_mut() = Some(projects.to_vec());
        Ok(())
    }

    fn save_workspace(
        &self,
        config_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<(), ConfigError> {
        self.recorder
            .workspaces
            .borrow_mut()
            .push((config_path.to_path_buf(), config.clone()));
        Ok(())
    }
}
