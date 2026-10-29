use std::{
    collections::{BTreeSet, HashMap},
    io,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError},
    thread,
    time::{Duration, Instant, SystemTime},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};

use super::{
    acceptor::Acceptor,
    event::ServiceEvent,
    holder_pool::HolderPool,
    located::Located,
    notice::Notice,
    notice_level::NoticeLevel,
    notices::{Notices, SharedNotices},
    process_action::ProcessAction,
    project_key::ProjectKey,
    project_ports::ProjectPorts,
    project_status::ProjectStatus,
    protocol,
    service_deps::ServiceDeps,
    service_error::ServiceError,
    service_error_body::ServiceErrorBody,
    service_error_kind::ServiceErrorKind,
    service_request::ServiceRequest,
    service_response::ServiceResponse,
    shared_holders::SharedHolders,
    singleton::Singleton,
    supervisor::{
        HolderScreen, ReopenOutcome, ScreenReply, StaleRelease, Supervisor, SupervisorDeps,
    },
};
use crate::{
    adapter::{bridge::WorkspaceRequest, holder::HolderRecord},
    domain::{
        config::ConfigError, port::ProcessHandle, pty::PtyError, settings::Settings,
        value::ProcessName,
    },
};

/// Most events waiting for the loop. Process output is most of them: a holder
/// connection whose output finds the queue full waits, so the holder's own
/// bounded backlog and resync take over instead of the service's memory. The
/// loop itself never sends here; launches, adoptions and screen reads post
/// from workers, so a full queue never blocks it.
pub(crate) const EVENT_CAPACITY: usize = 256;
/// Longest the service waits as it exits for launches and adoptions still in
/// flight, past the longest a launch can take: the holder ready timeout (5 s)
/// plus the time a failed launch spends stopping its holder (2 s).
const LAUNCH_DETACH_WAIT: Duration = Duration::from_secs(10);
/// Shown when a process name matches nothing in its project.
const UNKNOWN_PROCESS: &str = "no process with that name in this project";
/// Prefix of the refusal of a project path that is not absolute.
const RELATIVE_PROJECT: &str =
    "a project must be named by the absolute path of its muster.yml, not";
/// Shown when a project's config does not exist.
const UNKNOWN_PROJECT: &str = "no muster config at";
/// Shown when the process has no running child.
const NOT_RUNNING: &str = "the process is not running";
/// Why a session listing was refused when the session store cannot be read.
const SESSIONS_UNREADABLE: &str = "could not read this project's agent sessions";
/// Shown when a running process has no holder record to reach it by.
const NO_HOLDER: &str = "the process has no holder to reach it by";
/// Prefix of the notice for a holder record that cannot be read.
const UNREADABLE_RECORD_NOTICE: &str = "could not read the holder record";
/// What an unreadable record means for its process.
const UNREADABLE_RECORD_OUTCOME: &str =
    "its process, if it still runs, is left running unsupervised";
/// Prefix of the notice for a project whose running processes could not be
/// adopted because its config does not load.
const UNADOPTABLE_PROJECT_NOTICE: &str = "processes of this project are running but its config does not load; they are left running until it does";
/// Prefix of the notice for handover files that could not be removed.
const SWEEP_FAILED_NOTICE: &str = "could not remove the leftover holder handover file";
/// Notice when the logs an earlier muster kept per holder cannot be removed.
const LEGACY_LOGS_NOTICE: &str = "could not remove the old per-holder logs";

/// What handling one event leaves the loop to do.
enum Flow {
    /// Keep serving.
    Continue,
    /// Exit, then answer `reply` once the socket and lock are released.
    Exit(Sender<ServiceResponse>),
}

/// How a carried-out request is answered.
enum Reply {
    /// With this response, now.
    Now(ServiceResponse),
    /// With a process's screen; one only a holder can serve is read off the
    /// loop first.
    Screen(ScreenReply),
    /// Once the service has exited.
    Exit,
}

/// The one service of a user: owns every loaded project's supervisor on a
/// single thread, which every other thread only sends events to. Clients
/// connect on `service.sock`; holders run every process and outlive it.
pub struct ServiceRuntime {
    /// The running binary's version.
    version: String,
    /// The running binary's path.
    binary: PathBuf,
    /// The holders every process runs under.
    holders: Arc<dyn HolderPool>,
    /// Builds each project's driven adapters.
    ports: Box<dyn ProjectPorts>,
    /// The user's settings.
    settings: Settings,
    /// Where processes, timers, watchers, workers and clients deliver events.
    events: Sender<ServiceEvent>,
    /// Notices of every project and of the service.
    notices: SharedNotices,
    /// The supervisor of every loaded project.
    supervisors: HashMap<ProjectKey, Supervisor>,
}

impl ServiceRuntime {
    /// Runs the service until a client asks it to stop or be replaced: binds
    /// the socket in the singleton's state directory, adopts every running
    /// holder into its project's supervisor, and serves. On exit every holder
    /// keeps running, the socket is removed and the lock released before the
    /// exiting client is answered.
    ///
    /// # Errors
    /// Returns a [`ServiceError`] when the socket cannot be bound.
    pub fn run(singleton: Singleton, deps: ServiceDeps) -> Result<(), ServiceError> {
        let state_dir = singleton.state_dir().clone();
        let (sender, events) = bounded(EVENT_CAPACITY);
        let mut acceptor = Acceptor::bind(&protocol::socket_path(&state_dir), sender.clone())?;
        let mut runtime = Self::new(deps, sender, &state_dir);
        runtime.adopt_running();
        let exit = runtime.serve(&events);
        runtime.shutdown();
        acceptor.close();
        runtime.detach_launches(&events);
        runtime.supervisors.clear();
        drop(events);
        drop(singleton);
        if let Some(reply) = exit {
            let _ = reply.send(ServiceResponse::Ok);
        }
        acceptor.drain();
        Ok(())
    }

    /// A runtime over `deps` with no project loaded, mirroring its notices to
    /// the log in `state_dir`.
    fn new(deps: ServiceDeps, events: Sender<ServiceEvent>, state_dir: &Path) -> Self {
        let ServiceDeps {
            version,
            binary,
            holders,
            ports,
            settings,
        } = deps;
        Self {
            version,
            binary,
            holders,
            ports,
            settings,
            events,
            notices: Notices::shared(Some(protocol::log_path(state_dir))),
            supervisors: HashMap::new(),
        }
    }

    /// Takes over what a previous service left: removes the per-holder logs
    /// of an earlier muster and stale handover files, reports every holder
    /// record that cannot be read, and loads each project
    /// with running holders so they are adopted. A project whose config does
    /// not load is reported, and its holders are adopted once it does.
    fn adopt_running(&mut self) {
        if let Err(error) = self.holders.remove_legacy_logs() {
            self.notify(
                None,
                NoticeLevel::Warning,
                format!("{LEGACY_LOGS_NOTICE}: {error}"),
            );
        }
        match self.holders.sweep_handover_files() {
            Ok(failures) => {
                for (path, error) in failures {
                    self.notify(
                        None,
                        NoticeLevel::Warning,
                        format!("{SWEEP_FAILED_NOTICE} {}: {error}", path.display()),
                    );
                }
            },
            Err(error) => self.notify(
                None,
                NoticeLevel::Warning,
                format!("{SWEEP_FAILED_NOTICE}: {error}"),
            ),
        }
        for path in self.holders.unreadable_records() {
            self.notify(
                None,
                NoticeLevel::Error,
                format!(
                    "{UNREADABLE_RECORD_NOTICE} {}; {UNREADABLE_RECORD_OUTCOME}",
                    path.display()
                ),
            );
        }
        let records = self.holders.live_records();
        let projects: BTreeSet<PathBuf> = records
            .iter()
            .map(|record| record.project().clone())
            .collect();
        for project in projects {
            if let Err(refused) = self.load(&project, &records) {
                self.notify(
                    Some(ProjectKey::new(project)),
                    NoticeLevel::Error,
                    format!("{UNADOPTABLE_PROJECT_NOTICE}: {}", refused.message()),
                );
            }
        }
    }

    /// Handles events until a client asks the service to exit, waking for the
    /// supervisors' activity and metrics deadlines in between. Returns where
    /// to answer that client.
    fn serve(&mut self, events: &Receiver<ServiceEvent>) -> Option<Sender<ServiceResponse>> {
        loop {
            let received = match self.next_deadline() {
                Some(deadline) => events.recv_deadline(deadline),
                None => events.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(event) => {
                    if let Flow::Exit(reply) = self.handle(event) {
                        return Some(reply);
                    }
                },
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return None,
            }
            self.tick(Instant::now());
        }
    }

    /// The earliest activity or metrics deadline of any supervisor.
    fn next_deadline(&self) -> Option<Instant> {
        self.supervisors
            .values()
            .flat_map(|supervisor| {
                [
                    supervisor.next_activity_deadline(),
                    supervisor.next_metrics_deadline(),
                ]
            })
            .flatten()
            .min()
    }

    /// Acknowledges the held exits whose restart settled, then expires quiet
    /// activity and samples usage wherever they are due.
    fn tick(&mut self, now: Instant) {
        for supervisor in self.supervisors.values_mut() {
            supervisor.release_settled_exits();
            supervisor.expire_quiet_activity(now);
            supervisor.sample_metrics(now);
        }
    }

    /// Applies one event.
    fn handle(&mut self, event: ServiceEvent) -> Flow {
        match event {
            ServiceEvent::Output {
                project,
                pane,
                generation,
                output,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_output(pane, generation, output);
                }
            },
            ServiceEvent::Respawn {
                project,
                pane,
                generation,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_respawn(pane, generation);
                }
            },
            ServiceEvent::ForceStop {
                project,
                pane,
                spawn_generation,
                shutdown_generation,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_force_stop(pane, spawn_generation, shutdown_generation);
                }
            },
            ServiceEvent::Spawned {
                project,
                pane,
                generation,
                result,
            } => match self.supervisors.get_mut(&project) {
                Some(supervisor) => supervisor.handle_spawned(pane, generation, result),
                None => release_unrecorded(result),
            },
            ServiceEvent::Adopted {
                project,
                pane,
                generation,
                record,
                result,
            } => match self.supervisors.get_mut(&project) {
                Some(supervisor) => supervisor.handle_adopted(pane, generation, &record, result),
                None => release_unrecorded(result),
            },
            ServiceEvent::ConfigChanged { path } => {
                for supervisor in self.supervisors.values_mut() {
                    supervisor.handle_config_changed(&path);
                }
            },
            ServiceEvent::Request { request, reply } => return self.request(request, reply),
            ServiceEvent::HolderRead {
                screen,
                read,
                reply,
            } => {
                let _ = reply.send(ServiceResponse::Screen(self.settle_read(screen, read)));
            },
        }
        Flow::Continue
    }

    /// Answers one client request on `reply`: at once, once a worker has read
    /// a screen only a holder can serve, or after the service has exited.
    fn request(&mut self, request: ServiceRequest, reply: Sender<ServiceResponse>) -> Flow {
        let response = match self.respond(request) {
            Ok(Reply::Now(response)) => response,
            Ok(Reply::Screen(ScreenReply::Ready(screen))) => ServiceResponse::Screen(screen),
            Ok(Reply::Screen(ScreenReply::ReadHolder(screen))) => {
                self.read_off_loop(screen, reply);
                return Flow::Continue;
            },
            Ok(Reply::Exit) => return Flow::Exit(reply),
            Err(refusal) => ServiceResponse::Error(refusal),
        };
        let _ = reply.send(response);
        Flow::Continue
    }

    /// Carries out one client request.
    ///
    /// # Errors
    /// Returns the refusal to send when the project or process is unknown, or
    /// the request cannot be carried out.
    fn respond(&mut self, request: ServiceRequest) -> Result<Reply, ServiceErrorBody> {
        let response = match request {
            ServiceRequest::Replace | ServiceRequest::Stop => return Ok(Reply::Exit),
            ServiceRequest::Screen { project, name } => {
                let screen = self.process(&project, &name)?.screen(&name);
                return screen.map(Reply::Screen).ok_or_else(|| {
                    refused(
                        ServiceErrorKind::UnknownProcess,
                        format!("{UNKNOWN_PROCESS}: {name}"),
                    )
                });
            },
            ServiceRequest::Workspace {
                project,
                request: WorkspaceRequest::SendInput { name, data },
            } => {
                if !self
                    .process(&project, &name)?
                    .send_input(&name, data.as_bytes())
                {
                    return Err(refused(
                        ServiceErrorKind::NotRunning,
                        NOT_RUNNING.to_string(),
                    ));
                }
                ServiceResponse::Ok
            },
            ServiceRequest::Workspace {
                project,
                request: WorkspaceRequest::ListSessions,
            } => ServiceResponse::Sessions(self.supervisor(&project)?.sessions().map_err(
                |error| {
                    refused(
                        ServiceErrorKind::Internal,
                        format!("{SESSIONS_UNREADABLE}: {error}"),
                    )
                },
            )?),
            ServiceRequest::Hello { .. } => ServiceResponse::Hello {
                version: self.version.clone(),
                binary: self.binary.clone(),
            },
            ServiceRequest::Up { project } => {
                ServiceResponse::Batch(self.supervisor(&project)?.up())
            },
            ServiceRequest::Down { project, agents } => {
                ServiceResponse::Batch(self.supervisor(&project)?.down(agents))
            },
            ServiceRequest::Log { project, name } => {
                self.process(&project, &name)?;
                ServiceResponse::Log(self.holders.process_log(&project, &name))
            },
            ServiceRequest::Ps {
                project: Some(project),
            } => ServiceResponse::Status(vec![self.supervisor(&project)?.snapshot()]),
            ServiceRequest::Ps { project: None } => ServiceResponse::Status(self.statuses()),
            ServiceRequest::Control {
                project,
                name,
                action,
            } => {
                control(self.process(&project, &name)?, &name, action);
                ServiceResponse::Ok
            },
            ServiceRequest::Locate { project, name } => {
                ServiceResponse::Located(self.locate(&project, &name)?)
            },
            ServiceRequest::AgentNew {
                project,
                tool,
                name,
                command,
                resume,
            } => self
                .supervisor(&project)?
                .new_agent_session(tool, name, command, resume)
                .map(ServiceResponse::Agent)
                .map_err(|error| refused(ServiceErrorKind::Refused, error.to_string()))?,
            ServiceRequest::AgentClose { project, name } => {
                self.process(&project, &name)?.close_agent_session(&name);
                ServiceResponse::Ok
            },
            ServiceRequest::AgentReopen { project } => {
                match self.supervisor(&project)?.reopen_last_closed() {
                    Ok((name, ReopenOutcome::Opened(_))) => ServiceResponse::Agent(name),
                    Ok((name, ReopenOutcome::Queued)) => ServiceResponse::AgentQueued(name),
                    Err(error) => {
                        return Err(refused(ServiceErrorKind::Refused, error.to_string()));
                    },
                }
            },
            ServiceRequest::Notices { project } => {
                let key = project_key(&project)?;
                ServiceResponse::Notices(
                    self.notices
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .recent(&key),
                )
            },
        };
        Ok(Reply::Now(response))
    }

    /// Every loaded project's status, ordered by project.
    fn statuses(&self) -> Vec<ProjectStatus> {
        let mut statuses: Vec<_> = self
            .supervisors
            .values()
            .map(Supervisor::snapshot)
            .collect();
        statuses.sort_by(|left, right| left.project().as_ref().cmp(right.project().as_ref()));
        statuses
    }

    /// Where the running process `name` of `project` can be reached.
    ///
    /// # Errors
    /// Returns the refusal when the process is unknown, not running, or has
    /// no holder record.
    fn locate(&mut self, project: &Path, name: &ProcessName) -> Result<Located, ServiceErrorBody> {
        let pid = self
            .process(project, name)?
            .live_pid(name)
            .ok_or_else(|| refused(ServiceErrorKind::NotRunning, NOT_RUNNING.to_string()))?;
        let record = self
            .holders
            .locate(pid)
            .ok_or_else(|| refused(ServiceErrorKind::NotRunning, NO_HOLDER.to_string()))?;
        Ok(Located::builder()
            .socket(record.socket().clone())
            .log(record.log().clone())
            .build())
    }

    /// Reads `screen` from its holder on a worker thread, which posts the
    /// result back to the loop, so a slow holder never stalls the loop.
    fn read_off_loop(&self, screen: HolderScreen, reply: Sender<ServiceResponse>) {
        let events = self.events.clone();
        thread::spawn(move || {
            let read = screen.read();
            let _ = events.send(ServiceEvent::HolderRead {
                screen,
                read,
                reply,
            });
        });
    }

    /// Settles a holder screen read with its supervisor, or answers it as read
    /// when its project is no longer loaded.
    fn settle_read(&mut self, screen: HolderScreen, read: Result<String, PtyError>) -> String {
        match self.supervisors.get_mut(screen.project()) {
            Some(supervisor) => supervisor.settle_holder_read(screen, read),
            None => read.unwrap_or_else(|_| screen.fallback().clone()),
        }
    }

    /// The supervisor of `project` when it has a process named `name`.
    ///
    /// # Errors
    /// Returns the refusal when the project cannot be loaded, or an unknown
    /// process refusal listing the project's process names.
    fn process(
        &mut self,
        project: &Path,
        name: &ProcessName,
    ) -> Result<&mut Supervisor, ServiceErrorBody> {
        let supervisor = self.supervisor(project)?;
        if supervisor.has_process(name) {
            return Ok(supervisor);
        }
        Err(ServiceErrorBody::builder()
            .kind(ServiceErrorKind::UnknownProcess)
            .message(format!("{UNKNOWN_PROCESS}: {name}"))
            .candidates(supervisor.process_names())
            .build())
    }

    /// The supervisor of `project`, loading it on first use and adopting the
    /// holders already running its processes.
    ///
    /// # Errors
    /// Returns the refusal when the project's config is missing or invalid.
    fn supervisor(&mut self, project: &Path) -> Result<&mut Supervisor, ServiceErrorBody> {
        let key = project_key(project)?;
        if !self.supervisors.contains_key(&key) {
            let records = self.holders.live_records();
            self.load(project, &records)?;
        }
        self.supervisors
            .get_mut(&key)
            .ok_or_else(|| refused(ServiceErrorKind::UnknownProject, unknown_project(project)))
    }

    /// Loads `project`'s supervisor and adopts each of `records` that runs one
    /// of its processes.
    ///
    /// # Errors
    /// Returns the refusal when the project's config is missing or invalid.
    fn load(&mut self, project: &Path, records: &[HolderRecord]) -> Result<(), ServiceErrorBody> {
        let key = ProjectKey::new(project.to_path_buf());
        let shared = SharedHolders::new(Arc::clone(&self.holders));
        let deps = SupervisorDeps::builder()
            .runner(Box::new(shared.clone()))
            .registry(self.ports.registry())
            .agent_session_store(self.ports.agent_session_store())
            .events(self.events.clone())
            .notices(Arc::clone(&self.notices))
            .config_watcher(self.ports.config_watcher(self.events.clone()))
            .notifier(self.ports.notifier())
            .desktop_notifications(*self.settings.desktop_notifications())
            .process_metrics(self.ports.process_metrics())
            .size(*self.settings.default_size())
            .color_scheme(*self.settings.color_scheme())
            .build();
        let mut supervisor = Supervisor::load(key.clone(), deps)
            .map_err(|error| load_refusal(project, &error))?
            .with_adopter(Box::new(shared));
        for record in records
            .iter()
            .filter(|record| ProjectKey::new(record.project().clone()) == key)
        {
            supervisor.adopt(record.clone());
        }
        self.supervisors.insert(key, supervisor);
        Ok(())
    }

    /// Lets go of every process without stopping any.
    fn shutdown(&mut self) {
        for supervisor in self.supervisors.values_mut() {
            supervisor.shutdown();
        }
    }

    /// Waits, bounded by [`LAUNCH_DETACH_WAIT`], for every launch and
    /// adoption still in flight to come back, and hands each result to its
    /// supervisor, which detaches it. Exiting first would end a worker in the
    /// middle of greeting its holder, which then gives up and kills the child
    /// it was started for. Requests that arrive meanwhile are dropped, so
    /// their clients hear the service is going away.
    fn detach_launches(&mut self, events: &Receiver<ServiceEvent>) {
        let deadline = Instant::now() + LAUNCH_DETACH_WAIT;
        while self
            .supervisors
            .values()
            .any(|supervisor| supervisor.launches_in_flight() > 0)
        {
            match events.recv_deadline(deadline) {
                Ok(event @ (ServiceEvent::Spawned { .. } | ServiceEvent::Adopted { .. })) => {
                    self.handle(event);
                },
                Ok(_) => {},
                Err(_) => return,
            }
        }
    }

    /// Records a notice about `project`, or the service as a whole.
    fn notify(&self, project: Option<ProjectKey>, level: NoticeLevel, message: String) {
        let notice = Notice::builder()
            .at(SystemTime::now())
            .project(project)
            .process(None)
            .level(level)
            .message(message)
            .build();
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notice);
    }
}

/// Lets go of a launch or adoption result whose project has no supervisor:
/// no shutdown recorded that launch as one to detach, so its process is
/// discarded under the same decision a supervisor makes for a stale result.
fn release_unrecorded(result: Result<Box<dyn ProcessHandle>, PtyError>) {
    if let Ok(handle) = result {
        StaleRelease::Discard.release(handle);
    }
}

/// Applies `action` to the process `name` of `supervisor`.
fn control(supervisor: &mut Supervisor, name: &ProcessName, action: ProcessAction) {
    match action {
        ProcessAction::Start => supervisor.start(name),
        ProcessAction::Stop => supervisor.stop(name),
        ProcessAction::Restart => supervisor.restart(name),
        ProcessAction::Pause => supervisor.pause(name),
        ProcessAction::Resume => supervisor.resume(name),
        ProcessAction::Kill => supervisor.force_kill(name),
    }
}

/// A refusal of `kind` saying `message`.
fn refused(kind: ServiceErrorKind, message: String) -> ServiceErrorBody {
    ServiceErrorBody::builder()
        .kind(kind)
        .message(message)
        .build()
}

/// The key of the project whose config a client named at `project`, which
/// must be absolute: a relative path would resolve against the service's own
/// working directory, not the client's.
///
/// # Errors
/// Returns an invalid request refusal for a relative path.
fn project_key(project: &Path) -> Result<ProjectKey, ServiceErrorBody> {
    if project.is_absolute() {
        Ok(ProjectKey::new(project.to_path_buf()))
    } else {
        Err(refused(
            ServiceErrorKind::InvalidRequest,
            format!("{RELATIVE_PROJECT} {}", project.display()),
        ))
    }
}

/// The message for a project with no config at `project`.
fn unknown_project(project: &Path) -> String {
    format!("{UNKNOWN_PROJECT} {}", project.display())
}

/// The refusal for a project whose config failed to load with `error`: an
/// absent file is an unknown project, anything else an invalid config.
fn load_refusal(project: &Path, error: &ConfigError) -> ServiceErrorBody {
    match error {
        ConfigError::Read { source, .. } if source.kind() == io::ErrorKind::NotFound => {
            refused(ServiceErrorKind::UnknownProject, unknown_project(project))
        },
        error => refused(ServiceErrorKind::InvalidConfig, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::{
            fs::symlink,
            net::{UnixListener, UnixStream},
        },
        process,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread::JoinHandle,
        time::Duration,
    };

    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            config::load_workspace,
            holder::{
                ClientMessage, ClientRole, HolderConfig, HolderDirs, HolderError, HolderId,
                HolderLauncher, HolderRunner, HolderServer, abandon,
            },
            service::{
                BatchOutcome, ProcessStatus,
                supervisor::{
                    ChannelOutputSink, HolderAdopter, SpawnGeneration,
                    testing::{KillCountHandle, Orphan, UnreadableAgentSessionStore},
                },
            },
            terminal::SCROLLBACK_LINES,
        },
        domain::{
            color_scheme::ColorScheme,
            config::WorkspaceConfig,
            port::{
                AgentSessionStore, ConfigWatcher, Notifier, OutputSink, ProcessHandle,
                ProcessMetrics, ProcessRunner, ProjectRegistry,
            },
            process::{ProcessKind, ProcessState},
            project::Project,
            pty::{ProcessOutput, PtySize, SpawnRequest},
            value::{ByteSize, Cols, CommandLine, DetachKey, PaneId, Rows},
        },
    };

    /// Bound on every wait in these tests; reaching it fails the test.
    const WAIT: Duration = Duration::from_secs(10);
    /// Interval between checks of a polled condition.
    const POLL: Duration = Duration::from_millis(10);
    /// Short root for test sockets: macOS's long temp dir breaches `sun_path`.
    const SHORT_TEMP_ROOT: &str = "/tmp";
    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "ms-";
    /// The version the test services report.
    const VERSION: &str = "0.0.0-test";
    /// The binary the test services report running.
    const TEST_BINARY: &str = "/usr/bin/muster-test";
    /// A child that runs until it is killed.
    const SLEEPER: &str = "sleep 600";
    /// Longest the loop may take to answer a request while a holder read,
    /// launch or adoption is stalled, far below the holder timeouts.
    const PROMPT_ANSWER: Duration = Duration::from_secs(2);
    /// How long a gated holder waits after the service is told to stop,
    /// well inside the holder ready timeout.
    const GATE_DELAY: Duration = Duration::from_millis(300);
    /// A child that writes output as fast as it can.
    const FLOOD: &str = "yes muster-flood-line";
    /// How long a flood runs while nothing reads the queue.
    const FLOOD_HOLD: Duration = Duration::from_millis(800);
    /// How a resync screen starts: a full terminal reset.
    const RESYNC_PREFIX: &[u8] = b"\x1bc";

    /// Every holder thread a fixture's runners started.
    type Holders = Arc<Mutex<Vec<JoinHandle<Result<(), HolderError>>>>>;

    /// Runs each holder on a thread of the test process.
    struct ThreadLauncher(Holders);

    impl HolderLauncher for ThreadLauncher {
        fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
            let holder = thread::spawn(move || HolderServer::run(config));
            self.0.lock().unwrap().push(holder);
            Ok(())
        }
    }

    /// Runs each holder on a thread of the test process once `gate` opens or
    /// is dropped, as a holder that is slow to start does.
    struct GatedLauncher {
        holders: Holders,
        gate: Receiver<()>,
    }

    impl HolderLauncher for GatedLauncher {
        fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
            let gate = self.gate.clone();
            let holder = thread::spawn(move || {
                let _ = gate.recv_timeout(WAIT);
                HolderServer::run(config)
            });
            self.holders.lock().unwrap().push(holder);
            Ok(())
        }
    }

    /// Reads project configs straight from disk and writes nothing, so no
    /// test touches the user's registry.
    struct FileRegistry;

    impl ProjectRegistry for FileRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(Vec::new())
        }

        fn workspace(&self, config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            load_workspace(config_path)
        }

        fn workspace_exists(&self, config_path: &Path) -> bool {
            config_path.exists()
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Ok(())
        }
    }

    /// Ports over [`FileRegistry`] with nothing else wired, or with a session
    /// store that cannot be read when `unreadable_sessions` is set.
    #[derive(Default)]
    struct TestPorts {
        unreadable_sessions: bool,
    }

    impl ProjectPorts for TestPorts {
        fn registry(&self) -> Box<dyn ProjectRegistry> {
            Box::new(FileRegistry)
        }

        fn agent_session_store(&self) -> Option<Box<dyn AgentSessionStore>> {
            self.unreadable_sessions
                .then(|| Box::new(UnreadableAgentSessionStore) as Box<dyn AgentSessionStore>)
        }

        fn notifier(&self) -> Option<Box<dyn Notifier>> {
            None
        }

        fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
            None
        }

        fn config_watcher(&self, _events: Sender<ServiceEvent>) -> Option<Box<dyn ConfigWatcher>> {
            None
        }
    }

    /// A handle whose holder, when it has one, serves at `socket` but never
    /// answers, counting the kills it is sent.
    struct StalledHandle {
        socket: Option<PathBuf>,
        kills: Arc<AtomicUsize>,
    }

    impl ProcessHandle for StalledHandle {
        fn holder_socket(&self) -> Option<&Path> {
            self.socket.as_deref()
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

    /// A pool whose running holders are `records`, none of which it can
    /// adopt. Every spawn is counted and answers through a holder at `socket`
    /// that never replies. While `gate` is held, a launch or an adoption
    /// waits, as one for a holder that never becomes ready does.
    struct FakePool {
        records: Vec<HolderRecord>,
        spawns: Arc<AtomicUsize>,
        kills: Arc<AtomicUsize>,
        socket: Option<PathBuf>,
        gate: Option<Receiver<()>>,
        /// Times the legacy logs were removed.
        legacy_removals: Arc<AtomicUsize>,
        /// The failure the next legacy log removal reports.
        legacy_result: Mutex<Option<io::Error>>,
    }

    impl FakePool {
        /// A pool over `records` with no gate, whose holders serve at `socket`.
        fn new(records: Vec<HolderRecord>, socket: Option<PathBuf>) -> Self {
            Self {
                records,
                spawns: Arc::default(),
                kills: Arc::default(),
                socket,
                gate: None,
                legacy_removals: Arc::default(),
                legacy_result: Mutex::new(None),
            }
        }

        /// This pool, waiting in every launch and adoption until `gate`
        /// opens or is dropped.
        fn gated(self, gate: Receiver<()>) -> Self {
            Self {
                gate: Some(gate),
                ..self
            }
        }

        /// Waits, bounded by [`WAIT`], for the gate to open.
        fn wait_at_gate(&self) {
            if let Some(gate) = &self.gate {
                let _ = gate.recv_timeout(WAIT);
            }
        }
    }

    impl ProcessRunner for FakePool {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            self.wait_at_gate();
            Ok(Box::new(StalledHandle {
                socket: self.socket.clone(),
                kills: Arc::clone(&self.kills),
            }))
        }
    }

    impl HolderAdopter for FakePool {
        fn locate(&self, _child_pid: u32) -> Option<HolderRecord> {
            None
        }

        fn adopt(
            &self,
            _record: &HolderRecord,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            self.wait_at_gate();
            Err(PtyError::System("unreachable holder".to_string()))
        }
    }

    impl HolderPool for FakePool {
        fn live_records(&self) -> Vec<HolderRecord> {
            self.records.clone()
        }

        fn unreadable_records(&self) -> Vec<PathBuf> {
            Vec::new()
        }

        fn sweep_handover_files(&self) -> io::Result<Vec<(PathBuf, io::Error)>> {
            Ok(Vec::new())
        }

        fn process_log(&self, project: &Path, name: &ProcessName) -> PathBuf {
            project.with_file_name(format!("{name}.log"))
        }

        fn remove_legacy_logs(&self) -> io::Result<()> {
            self.legacy_removals.fetch_add(1, Ordering::SeqCst);
            self.legacy_result
                .lock()
                .unwrap()
                .take()
                .map_or(Ok(()), Err)
        }
    }

    /// Discards a detached handle's output.
    struct NullSink;

    impl OutputSink for NullSink {
        fn send(&self, _output: ProcessOutput) {}
    }

    /// Settings with every value explicit and desktop notifications off.
    fn settings() -> Settings {
        Settings::builder()
            .desktop_notifications(false)
            .default_size(size())
            .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
            .log_max_size(ByteSize::try_new(RawByteSize::mib(4)).unwrap())
            .scrollback(SCROLLBACK_LINES)
            .color_scheme(ColorScheme::Dark)
            .build()
    }

    /// The terminal size every test process starts with.
    fn size() -> PtySize {
        PtySize::builder()
            .rows(Rows::new(24))
            .cols(Cols::new(80))
            .build()
    }

    /// Polls `condition` until it holds, failing the test at [`WAIT`].
    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
            thread::sleep(POLL);
        }
    }

    /// A scratch state directory, holder dirs and project, plus every holder
    /// thread started there; dropping it kills every child still recorded and
    /// lets its holder finish.
    struct Fixture {
        scratch: TempDir,
        holders: Holders,
    }

    impl Fixture {
        /// A fresh scratch directory short enough to bind sockets in.
        fn new() -> Self {
            Self {
                scratch: tempfile::Builder::new()
                    .prefix(SCRATCH_PREFIX)
                    .tempdir_in(SHORT_TEMP_ROOT)
                    .unwrap(),
                holders: Arc::default(),
            }
        }

        /// The state directory the test services own.
        fn state_dir(&self) -> PathBuf {
            self.scratch.path().join("state")
        }

        /// Where this fixture's holders keep their sockets, records and logs.
        fn dirs(&self) -> HolderDirs {
            let root = self.scratch.path().join("h");
            HolderDirs::builder()
                .sockets(root.join("sock"))
                .records(root.join("records"))
                .logs(root.join("logs"))
                .build()
        }

        /// A runner launching in-thread holders into this fixture's dirs, as a
        /// service has.
        fn runner(&self) -> HolderRunner {
            self.runner_with(Box::new(ThreadLauncher(Arc::clone(&self.holders))))
        }

        /// As [`Self::runner`], whose holders start only once `gate` opens.
        fn gated_runner(&self, gate: Receiver<()>) -> HolderRunner {
            self.runner_with(Box::new(GatedLauncher {
                holders: Arc::clone(&self.holders),
                gate,
            }))
        }

        /// A runner launching through `launcher` into this fixture's dirs.
        fn runner_with(&self, launcher: Box<dyn HolderLauncher>) -> HolderRunner {
            HolderRunner::new(
                launcher,
                self.dirs(),
                ByteSize::try_new(RawByteSize::mib(4)).unwrap(),
                SCROLLBACK_LINES,
                ColorScheme::Dark,
            )
        }

        /// Writes `config` as the project's `muster.yml` and returns its path.
        fn project(&self, config: &str) -> PathBuf {
            self.project_in("project", config)
        }

        /// Writes `config` as the `muster.yml` of the project in `folder` and
        /// returns its path.
        fn project_in(&self, folder: &str, config: &str) -> PathBuf {
            let dir = self.scratch.path().join(folder);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("muster.yml");
            fs::write(&path, config).unwrap();
            path
        }

        /// A request for `process` of kind `command` in `project`.
        fn request(&self, project: &Path, process: &str, command: &str) -> SpawnRequest {
            SpawnRequest::builder()
                .process(ProcessName::try_new(process).unwrap())
                .kind(ProcessKind::Command)
                .command(Some(CommandLine::try_new(command).unwrap()))
                .project(Some(project.to_path_buf()))
                .working_dir(Some(self.scratch.path().to_path_buf()))
                .size(size())
                .build()
        }

        /// Starts a service over `pool` and waits until it accepts clients.
        fn start(&self, pool: Arc<dyn HolderPool>) -> Running {
            self.start_with(pool, TestPorts::default())
        }

        /// Starts a service over `pool` and `ports` and waits until it
        /// accepts clients.
        fn start_with(&self, pool: Arc<dyn HolderPool>, ports: TestPorts) -> Running {
            let state_dir = self.state_dir();
            let socket = protocol::socket_path(&state_dir);
            let thread = thread::spawn(move || {
                let Some(singleton) = Singleton::acquire(&state_dir)? else {
                    return Ok(());
                };
                let deps = ServiceDeps::builder()
                    .version(VERSION.to_string())
                    .binary(PathBuf::from(TEST_BINARY))
                    .holders(pool)
                    .ports(Box::new(ports))
                    .settings(settings())
                    .build();
                ServiceRuntime::run(singleton, deps)
            });
            wait_until("the service to accept", || {
                UnixStream::connect(&socket).is_ok()
            });
            Running {
                socket,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Fixture {
        /// Kills every child still recorded and collects its exit so its
        /// holder finishes, then joins the holder threads, bounded by [`WAIT`].
        fn drop(&mut self) {
            let dirs = self.dirs();
            for record in HolderRecord::load_all(dirs.records()) {
                if record.child_pid() != process::id() {
                    // SAFETY: `kill` takes plain integers and touches no
                    // memory; a stale pid at worst signals nothing.
                    unsafe {
                        libc::kill(-(record.child_pid() as libc::pid_t), libc::SIGKILL);
                    }
                }
                abandon(record.socket());
            }
            let started = Instant::now();
            let holders = std::mem::take(&mut *self.holders.lock().unwrap());
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

    /// A service running on a thread of the test.
    struct Running {
        socket: PathBuf,
        thread: Option<JoinHandle<Result<(), ServiceError>>>,
    }

    impl Running {
        /// Sends `request` as a client would and returns the answer.
        fn send(&self, request: ServiceRequest) -> ServiceResponse {
            protocol::send(&self.socket, &request).unwrap()
        }

        /// Sends `Stop`, then waits for the service to exit.
        fn stop(mut self) {
            assert_eq!(self.send(ServiceRequest::Stop), ServiceResponse::Ok);
            self.join();
        }

        /// Waits, bounded, for the service thread to finish and checks it
        /// exited cleanly.
        fn join(&mut self) {
            let Some(thread) = self.thread.take() else {
                return;
            };
            wait_until("the service to exit", || thread.is_finished());
            thread.join().unwrap().unwrap();
        }

        /// Polls `ps` until the process `name` of `project` is in `state`,
        /// failing at [`WAIT`], and returns that status.
        fn wait_for(&self, project: &Path, name: &str, state: ProcessState) -> ProjectStatus {
            let mut status = self.status(project);
            wait_until("the process to reach its state", || {
                status = self.status(project);
                *process(&status, name).state() == state
            });
            status
        }

        /// The one project status a `ps` of `project` reports.
        fn status(&self, project: &Path) -> ProjectStatus {
            let ServiceResponse::Status(mut statuses) = self.send(ServiceRequest::Ps {
                project: Some(project.to_path_buf()),
            }) else {
                panic!("ps answers a status");
            };
            statuses.remove(0)
        }
    }

    impl Drop for Running {
        /// Stops a service a failing test left running.
        fn drop(&mut self) {
            if self.thread.is_some() {
                let _ = protocol::send(&self.socket, &ServiceRequest::Stop);
                if let Some(thread) = self.thread.take() {
                    let started = Instant::now();
                    while !thread.is_finished() && started.elapsed() < WAIT {
                        thread::sleep(POLL);
                    }
                }
            }
        }
    }

    /// A project of commands, each `(name, autostart)` running [`SLEEPER`].
    fn commands(entries: &[(&str, bool)]) -> String {
        let commands: String = entries
            .iter()
            .map(|(name, autostart)| {
                format!("  - name: {name}\n    command: {SLEEPER}\n    autostart: {autostart}\n")
            })
            .collect();
        format!("agents: []\nterminals: []\ncommands:\n{commands}")
    }

    /// The status of the process `name` in `status`.
    fn process<'a>(status: &'a ProjectStatus, name: &str) -> &'a ProcessStatus {
        status
            .processes()
            .iter()
            .find(|process| process.name().as_ref() == name)
            .unwrap()
    }

    #[test]
    fn service_adopts_live_holders_on_start_and_ps_reports_them_running() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("dev server", false)]));
        let handle = fixture
            .runner()
            .spawn(
                fixture.request(&project, "dev server", SLEEPER),
                Box::new(NullSink),
            )
            .unwrap();
        let pid = handle.process_id().unwrap();
        drop(handle);

        let service = fixture.start(Arc::new(fixture.runner()));

        let ServiceResponse::Status(loaded) = service.send(ServiceRequest::Ps { project: None })
        else {
            panic!("ps answers a status");
        };
        assert_eq!(
            loaded.len(),
            1,
            "the project was loaded to adopt its holder"
        );
        let status = service.wait_for(&project, "dev server", ProcessState::Running);
        assert_eq!(*process(&status, "dev server").pid(), Some(pid));
        service.stop();
    }

    #[test]
    fn stop_request_exits_the_loop_and_holders_keep_running() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });
        let running = service.wait_for(&project, "api", ProcessState::Running);
        let pid = process(&running, "api").pid().unwrap();
        let socket = service.socket.clone();

        service.stop();

        assert!(!socket.exists(), "the socket is removed");
        assert!(
            Singleton::acquire(&fixture.state_dir()).unwrap().is_some(),
            "the lock is released"
        );
        let live = fixture.runner().live_records();
        assert_eq!(live.len(), 1, "the holder keeps running");
        assert_eq!(live[0].child_pid(), pid);

        let successor = fixture.start(Arc::new(fixture.runner()));
        let api = successor.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(*process(&api, "api").state(), ProcessState::Running);
        assert_eq!(*process(&api, "api").pid(), Some(pid), "the same child");
        successor.stop();
    }

    #[test]
    fn unknown_process_error_lists_candidates() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("dev server", false), ("café", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));

        let response = service.send(ServiceRequest::Control {
            project,
            name: ProcessName::try_new("nope").unwrap(),
            action: ProcessAction::Start,
        });

        let ServiceResponse::Error(refusal) = response else {
            panic!("an unknown name is refused, got {response:?}");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::UnknownProcess);
        assert_eq!(refusal.candidates(), &vec![
            ProcessName::try_new("dev server").unwrap(),
            ProcessName::try_new("café").unwrap(),
        ]);
        service.stop();
    }

    #[test]
    fn up_loads_an_unloaded_project_and_starts_only_autostart_true() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("eager", true), ("lazy", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        assert_eq!(
            service.send(ServiceRequest::Ps { project: None }),
            ServiceResponse::Status(Vec::new()),
            "nothing is loaded before it is asked for"
        );

        assert_eq!(
            service.send(ServiceRequest::Up {
                project: project.clone()
            }),
            ServiceResponse::Batch(
                BatchOutcome::builder()
                    .acted(vec![ProcessName::try_new("eager").unwrap()])
                    .refused(vec![])
                    .build()
            )
        );

        let status = service.wait_for(&project, "eager", ProcessState::Running);
        assert!(process(&status, "eager").pid().is_some());
        assert_eq!(*process(&status, "lazy").state(), ProcessState::Pending);
        assert_eq!(fixture.runner().live_records().len(), 1);
        service.stop();
    }

    /// A project whose config is missing or invalid is refused with its kind,
    /// and the invalid config's message names the offending process.
    #[test]
    fn a_missing_or_invalid_project_is_refused_with_its_reason() {
        let fixture = Fixture::new();
        let service = fixture.start(Arc::new(fixture.runner()));

        let missing = service.send(ServiceRequest::Up {
            project: fixture.scratch.path().join("absent/muster.yml"),
        });
        let ServiceResponse::Error(missing) = missing else {
            panic!("a missing project is refused");
        };
        assert_eq!(*missing.kind(), ServiceErrorKind::UnknownProject);

        let invalid = fixture.project("agents: []\nterminals: []\ncommands:\n  - name: api\n");
        let ServiceResponse::Error(refusal) = service.send(ServiceRequest::Up { project: invalid })
        else {
            panic!("an invalid project is refused");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::InvalidConfig);
        assert!(refusal.message().contains("'api'"), "{}", refusal.message());
        service.stop();
    }

    /// Input is answered with `Ok` once delivered, and with the standard
    /// refusals for a stopped process and an unknown name, which lists the
    /// valid ones.
    #[test]
    fn input_is_delivered_or_refused_like_any_process_request() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        let send = |name: &str| {
            service.send(ServiceRequest::Workspace {
                project: project.clone(),
                request: WorkspaceRequest::SendInput {
                    name: ProcessName::try_new(name).unwrap(),
                    data: "hello\r".to_string(),
                },
            })
        };

        let ServiceResponse::Error(stopped) = send("api") else {
            panic!("a stopped process takes no input");
        };
        assert_eq!(*stopped.kind(), ServiceErrorKind::NotRunning);
        let ServiceResponse::Error(unknown) = send("web") else {
            panic!("an unknown name takes no input");
        };
        assert_eq!(*unknown.kind(), ServiceErrorKind::UnknownProcess);
        assert_eq!(
            unknown.candidates(),
            &[ProcessName::try_new("api").unwrap()]
        );

        service.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new("api").unwrap(),
            action: ProcessAction::Start,
        });
        service.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(send("api"), ServiceResponse::Ok);
        service.stop();
    }

    /// A session store that cannot be read is refused as the service's own
    /// failure, with the reason, never answered as an empty listing.
    #[test]
    fn an_unreadable_session_store_is_refused() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", false)]));
        let service = fixture.start_with(Arc::new(fixture.runner()), TestPorts {
            unreadable_sessions: true,
        });

        let ServiceResponse::Error(refusal) = service.send(ServiceRequest::Workspace {
            project,
            request: WorkspaceRequest::ListSessions,
        }) else {
            panic!("an unreadable store is refused");
        };

        assert_eq!(*refusal.kind(), ServiceErrorKind::Internal);
        assert!(
            refusal.message().starts_with(SESSIONS_UNREADABLE),
            "{}",
            refusal.message()
        );
        service.stop();
    }

    /// `Locate` names the holder socket and log of a running process, and a
    /// writable client attaching there is counted as the holder reports it,
    /// down again once its connection closes.
    #[test]
    fn locate_names_the_holder_and_attached_is_counted() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });
        service.wait_for(&project, "api", ProcessState::Running);
        let name = ProcessName::try_new("api").unwrap();

        let ServiceResponse::Located(located) = service.send(ServiceRequest::Locate {
            project: project.clone(),
            name: name.clone(),
        }) else {
            panic!("a running process is located");
        };
        let record = &fixture.runner().live_records()[0];
        assert_eq!(located.socket(), record.socket());
        assert_eq!(located.log(), record.log());

        let mut attach = UnixStream::connect(located.socket()).unwrap();
        ClientMessage::Hello {
            role: ClientRole::Attach { read_only: false },
        }
        .write_to(&mut attach)
        .unwrap();
        wait_until("the holder to report the attach", || {
            *process(&service.status(&project), "api").attached() == 1
        });
        drop(attach);
        wait_until("the holder to report the detach", || {
            *process(&service.status(&project), "api").attached() == 0
        });

        service.send(ServiceRequest::Control {
            project: project.clone(),
            name: name.clone(),
            action: ProcessAction::Kill,
        });
        wait_until("the kill to settle", || {
            !process(&service.status(&project), "api")
                .state()
                .may_be_running()
        });
        let ServiceResponse::Error(refusal) =
            service.send(ServiceRequest::Locate { project, name })
        else {
            panic!("a stopped process cannot be located");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::NotRunning);
        service.stop();
    }

    /// A holder screen is read off the loop: while one holder never answers,
    /// the service still answers other requests at once, and the stalled read
    /// is answered with the service's own copy once it gives up.
    #[test]
    fn a_stalled_holder_screen_read_never_stalls_the_loop() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let stalled = fixture.scratch.path().join("stalled.sock");
        let _never_accepts = UnixListener::bind(&stalled).unwrap();
        let pool = FakePool::new(Vec::new(), Some(stalled));
        let service = fixture.start(Arc::new(pool));
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });

        let socket = service.socket.clone();
        service.wait_for(&project, "api", ProcessState::Running);
        let screen_project = project.clone();
        let reading = thread::spawn(move || {
            protocol::send(&socket, &ServiceRequest::Screen {
                project: screen_project,
                name: ProcessName::try_new("api").unwrap(),
            })
            .unwrap()
        });
        thread::sleep(POLL * 10);

        let asked = Instant::now();
        let status = service.status(&project);
        assert!(asked.elapsed() < PROMPT_ANSWER, "the loop answered at once");
        assert_eq!(*process(&status, "api").state(), ProcessState::Running);
        assert!(!reading.is_finished(), "the holder read is still stalled");

        wait_until("the stalled read to give up", || reading.is_finished());
        assert!(matches!(
            reading.join().unwrap(),
            ServiceResponse::Screen(_)
        ));
        service.stop();
    }

    /// At start the service removes handover files no holder will read and
    /// reports every holder record it cannot read, naming the file.
    #[test]
    fn start_sweeps_handover_files_and_reports_unreadable_records() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", false)]));
        let dirs = fixture.dirs();
        dirs.prepare_sockets().unwrap();
        let id = HolderId::generate().unwrap();
        let spec = dirs.socket_path(&id).with_extension("json");
        let error = dirs.socket_path(&id).with_extension("err");
        fs::write(&spec, b"{\"environment\":\"secret\"}").unwrap();
        fs::write(&error, b"failed").unwrap();
        fs::create_dir_all(dirs.records()).unwrap();
        let unreadable = dirs.records().join("corrupt.json");
        fs::write(&unreadable, b"{not a record").unwrap();

        let service = fixture.start(Arc::new(fixture.runner()));

        assert!(
            !spec.exists() && !error.exists(),
            "handover files are swept"
        );
        let ServiceResponse::Notices(notices) = service.send(ServiceRequest::Notices { project })
        else {
            panic!("notices answer notices");
        };
        assert!(
            notices.iter().any(|notice| {
                notice.project().is_none()
                    && *notice.level() == NoticeLevel::Error
                    && notice.message().contains(&unreadable.display().to_string())
            }),
            "{notices:?}"
        );
        service.stop();
    }

    /// A start removes the legacy per-holder logs once, and a failure to is
    /// a service-wide warning rather than a reason not to start.
    #[test]
    fn start_removes_the_legacy_logs_and_reports_a_failure() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", false)]));
        let pool = FakePool::new(Vec::new(), None);
        *pool.legacy_result.lock().unwrap() = Some(io::Error::other("read-only file system"));
        let removals = Arc::clone(&pool.legacy_removals);

        let service = fixture.start(Arc::new(pool));

        assert_eq!(removals.load(Ordering::SeqCst), 1);
        let ServiceResponse::Notices(notices) = service.send(ServiceRequest::Notices { project })
        else {
            panic!("notices answer notices");
        };
        assert!(
            notices.iter().any(|notice| {
                notice.project().is_none()
                    && *notice.level() == NoticeLevel::Warning
                    && notice.message().contains(LEGACY_LOGS_NOTICE)
                    && notice.message().contains("read-only file system")
            }),
            "{notices:?}"
        );
        service.stop();
    }

    /// The record a holder running the command `api` of `project` as
    /// `child_pid` would leave, its socket gone.
    fn api_record(fixture: &Fixture, project: &Path, child_pid: u32) -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(project.to_path_buf())
            .process(ProcessName::try_new("api").unwrap())
            .kind(ProcessKind::Command)
            .agent_session_id(None)
            .socket(fixture.scratch.path().join("gone.sock"))
            .holder_pid(process::id())
            .child_pid(child_pid)
            .child_start_token(None)
            .log(fixture.scratch.path().join("api.log"))
            .child_started_at(SystemTime::now())
            .build()
    }

    /// A holder still running a process that cannot be adopted leaves the
    /// process unsupervised, and neither `up` nor `start` launches a second
    /// copy beside it.
    #[test]
    fn a_holder_that_cannot_be_adopted_blocks_up_and_start() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let pool = FakePool::new(vec![api_record(&fixture, &project, orphan.pid())], None);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        let name = ProcessName::try_new("api").unwrap();

        service.wait_for(&project, "api", ProcessState::Unsupervised);
        let ServiceResponse::Batch(up) = service.send(ServiceRequest::Up {
            project: project.clone(),
        }) else {
            panic!("up answers what it did");
        };
        assert!(up.acted().is_empty(), "{up:?}");
        assert_eq!(up.refused().len(), 1, "{up:?}");
        assert_eq!(up.refused()[0].name(), &name);
        assert!(
            up.refused()[0]
                .reason()
                .ends_with(&orphan.pid().to_string()),
            "up says it left api alone and why: {up:?}"
        );
        service.send(ServiceRequest::Control {
            project: project.clone(),
            name,
            action: ProcessAction::Start,
        });

        let status = service.status(&project);
        assert_eq!(*process(&status, "api").state(), ProcessState::Unsupervised);
        assert_eq!(*process(&status, "api").pid(), Some(orphan.pid()));
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");
        let ServiceResponse::Notices(notices) = service.send(ServiceRequest::Notices { project })
        else {
            panic!("notices answer notices");
        };
        assert!(
            notices
                .iter()
                .any(|notice| notice.message().ends_with(&orphan.pid().to_string())),
            "the refused start names the pid: {notices:?}"
        );
        service.stop();
    }

    /// A launch whose holder never becomes ready runs off the loop: the
    /// process shows `starting`, and a request for another project is
    /// answered at once meanwhile.
    #[test]
    fn a_holder_that_never_becomes_ready_does_not_delay_another_project() {
        let fixture = Fixture::new();
        let slow = fixture.project_in("slow", &commands(&[("api", true)]));
        let other = fixture.project_in("other", &commands(&[("web", false)]));
        let (open, gate) = bounded(0);
        let service = fixture.start(Arc::new(FakePool::new(Vec::new(), None).gated(gate)));

        service.send(ServiceRequest::Up {
            project: slow.clone(),
        });
        let asked = Instant::now();
        let status = service.status(&other);
        assert!(asked.elapsed() < PROMPT_ANSWER, "the loop answered at once");
        assert_eq!(*process(&status, "web").state(), ProcessState::Pending);
        assert_eq!(
            *process(&service.status(&slow), "api").state(),
            ProcessState::Starting
        );

        drop(open);
        service.wait_for(&slow, "api", ProcessState::Running);
        service.stop();
    }

    /// A stop or kill while a launch is in flight is kept for it: once the
    /// process arrives it is signalled at once, so nothing is left running.
    #[test]
    fn stop_or_kill_during_starting_leaves_no_running_child() {
        for action in [ProcessAction::Stop, ProcessAction::Kill] {
            let fixture = Fixture::new();
            let project = fixture.project(&commands(&[("api", true)]));
            let (open, gate) = bounded(0);
            let pool = FakePool::new(Vec::new(), None).gated(gate);
            let kills = Arc::clone(&pool.kills);
            let service = fixture.start(Arc::new(pool));
            service.send(ServiceRequest::Up {
                project: project.clone(),
            });
            service.wait_for(&project, "api", ProcessState::Starting);

            service.send(ServiceRequest::Control {
                project: project.clone(),
                name: ProcessName::try_new("api").unwrap(),
                action,
            });
            drop(open);

            wait_until("the launched process to be signalled", || {
                kills.load(Ordering::SeqCst) == 1
            });
            assert_eq!(
                *process(&service.status(&project), "api").state(),
                ProcessState::Stopping,
                "{action:?} reached the process, which is going"
            );
            service.stop();
        }
    }

    /// A stop while the adoption of a running child is in flight is kept for
    /// it: when the adoption fails the child is left unsupervised and the stop
    /// goes on through the unsupervised path until the child is gone, and no
    /// start launches a second copy meanwhile.
    #[test]
    fn stop_during_a_failing_adoption_stops_the_child_and_starts_nothing() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let (open, gate) = bounded(0);
        let pool =
            FakePool::new(vec![api_record(&fixture, &project, orphan.pid())], None).gated(gate);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        let name = ProcessName::try_new("api").unwrap();
        service.wait_for(&project, "api", ProcessState::Starting);

        for action in [ProcessAction::Stop, ProcessAction::Start] {
            service.send(ServiceRequest::Control {
                project: project.clone(),
                name: name.clone(),
                action,
            });
        }
        let ServiceResponse::Ok = service.send(ServiceRequest::Control {
            project: project.clone(),
            name: name.clone(),
            action: ProcessAction::Stop,
        }) else {
            panic!("the stop is accepted while starting");
        };
        drop(open);

        service.wait_for(&project, "api", ProcessState::Stopping);
        service.send(ServiceRequest::Control {
            project: project.clone(),
            name,
            action: ProcessAction::Start,
        });
        wait_until("the unsupervised child to be stopped", || !orphan.alive());
        service.wait_for(&project, "api", ProcessState::Exited);
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");
        service.stop();
    }

    /// A launch still in flight when the service stops is not cancelled: the
    /// service waits for it, detaches it, and its holder and child live on
    /// for the next service, which adopts the process as running.
    #[test]
    fn a_launch_in_flight_at_service_stop_survives_and_is_adopted() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let (open, gate) = bounded(0);
        let service = fixture.start(Arc::new(fixture.gated_runner(gate)));
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });
        service.wait_for(&project, "api", ProcessState::Starting);

        let opener = thread::spawn(move || {
            thread::sleep(GATE_DELAY);
            let _ = open.send(());
        });
        service.stop();
        let live = fixture.runner().live_records();
        assert_eq!(
            live.len(),
            1,
            "the service waited for the launch before it exited, and its holder lives on"
        );
        opener.join().unwrap();
        let pid = live[0].child_pid();
        // SAFETY: signal 0 only checks that the pid exists.
        assert_eq!(
            unsafe { libc::kill(pid as libc::pid_t, 0) },
            0,
            "the child lives on"
        );
        let successor = fixture.start(Arc::new(fixture.runner()));
        let status = successor.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(*process(&status, "api").pid(), Some(pid));
        successor.stop();
    }

    /// Starting a process already starting launches nothing more.
    #[test]
    fn start_during_starting_does_not_double_spawn() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let (open, gate) = bounded(0);
        let pool = FakePool::new(Vec::new(), None).gated(gate);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        let name = ProcessName::try_new("api").unwrap();
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });
        service.wait_for(&project, "api", ProcessState::Starting);

        service.send(ServiceRequest::Control {
            project: project.clone(),
            name: name.clone(),
            action: ProcessAction::Start,
        });
        service.send(ServiceRequest::Up {
            project: project.clone(),
        });
        drop(open);

        service.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "one launch only");
        service.stop();
    }

    /// Adopting a holder that never answers runs off the loop: the service
    /// answers at once while the process shows `starting`, and once the
    /// adoption fails the still-running child is left unsupervised.
    #[test]
    fn startup_adoption_of_a_wedged_holder_does_not_delay_requests() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let (open, gate) = bounded(0);
        let pool =
            FakePool::new(vec![api_record(&fixture, &project, orphan.pid())], None).gated(gate);
        let service = fixture.start(Arc::new(pool));

        let asked = Instant::now();
        let status = service.status(&project);
        assert!(asked.elapsed() < PROMPT_ANSWER, "the loop answered at once");
        assert_eq!(*process(&status, "api").state(), ProcessState::Starting);

        drop(open);
        let status = service.wait_for(&project, "api", ProcessState::Unsupervised);
        assert_eq!(*process(&status, "api").pid(), Some(orphan.pid()));
        service.stop();
    }

    /// A process flooding output while the loop does not read fills the event
    /// queue to its capacity and no further: its holder connection waits, and
    /// the holder resyncs it with a fresh screen once it reads again.
    #[test]
    fn a_flood_waits_on_the_bounded_queue_and_the_holder_resyncs() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("flood", false)]));
        let (sender, events) = bounded(EVENT_CAPACITY);
        let sink = ChannelOutputSink::new(
            ProjectKey::new(project.clone()),
            PaneId::new(0),
            SpawnGeneration::initial(),
            sender,
        );
        let mut handle = fixture
            .runner()
            .spawn(fixture.request(&project, "flood", FLOOD), Box::new(sink))
            .unwrap();

        thread::sleep(FLOOD_HOLD);
        assert_eq!(
            events.len(),
            EVENT_CAPACITY,
            "the queue is full and no fuller"
        );

        let started = Instant::now();
        let resynced = loop {
            assert!(started.elapsed() < WAIT, "timed out waiting for a resync");
            match events.recv_timeout(WAIT) {
                Ok(ServiceEvent::Output {
                    output: ProcessOutput::Chunk(bytes),
                    ..
                }) if bytes.starts_with(RESYNC_PREFIX) => break true,
                Ok(_) => assert!(events.len() <= EVENT_CAPACITY),
                Err(_) => break false,
            }
        };
        assert!(
            resynced,
            "the holder resynced the connection it could not feed"
        );
        handle.kill().unwrap();
    }

    /// A project named by a relative path is refused, since it would resolve
    /// against the service's directory, and one `muster.yml` reached through a
    /// symlink is the same project as through its own path.
    #[test]
    fn projects_are_named_absolutely_and_aliases_are_one_project() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", false)]));
        let alias = fixture.scratch.path().join("alias");
        symlink(project.parent().unwrap(), &alias).unwrap();
        let service = fixture.start(Arc::new(fixture.runner()));

        let ServiceResponse::Error(refusal) = service.send(ServiceRequest::Ps {
            project: Some(PathBuf::from("project/muster.yml")),
        }) else {
            panic!("a relative project is refused");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::InvalidRequest);

        service.status(&project);
        service.status(&alias.join("muster.yml"));
        let ServiceResponse::Status(loaded) = service.send(ServiceRequest::Ps { project: None })
        else {
            panic!("ps answers a status");
        };
        assert_eq!(loaded.len(), 1, "both paths name one project");
        service.stop();
    }

    /// A project with running holders whose config does not load still
    /// answers its notices, which say why its processes were left alone.
    #[test]
    fn notices_of_a_project_whose_config_fails_to_load_are_answered() {
        let fixture = Fixture::new();
        let project = fixture.project("agents: []\nterminals: []\ncommands:\n  - name: api\n");
        let orphan = Orphan::spawn();
        let pool = FakePool::new(vec![api_record(&fixture, &project, orphan.pid())], None);
        let service = fixture.start(Arc::new(pool));

        let ServiceResponse::Notices(notices) = service.send(ServiceRequest::Notices { project })
        else {
            panic!("notices of an unloadable project are answered");
        };
        assert!(
            notices
                .iter()
                .any(|notice| notice.message().starts_with(UNADOPTABLE_PROJECT_NOTICE)),
            "{notices:?}"
        );
        service.stop();
    }

    /// A client is told the service's own version and binary, so a newer
    /// client can replace it and an older one can say which muster it met.
    #[test]
    fn hello_answers_the_service_version() {
        let fixture = Fixture::new();
        let service = fixture.start(Arc::new(fixture.runner()));

        assert_eq!(
            service.send(ServiceRequest::Hello {
                version: "9.9.9".to_string()
            }),
            ServiceResponse::Hello {
                version: VERSION.to_string(),
                binary: PathBuf::from(TEST_BINARY),
            }
        );
        service.stop();
    }

    /// `Up` and `Down` answer with what they started or stopped and what
    /// they left alone, and the log of a process is known whether or not it
    /// runs.
    #[test]
    fn up_and_down_say_what_they_did_and_logs_are_found_for_stopped_processes() {
        let fixture = Fixture::new();
        let project = fixture.project(&commands(&[("api", true), ("worker", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));

        let ServiceResponse::Batch(up) = service.send(ServiceRequest::Up {
            project: project.clone(),
        }) else {
            panic!("up answers what it did");
        };
        assert_eq!(up.acted(), &vec![ProcessName::try_new("api").unwrap()]);
        assert!(up.refused().is_empty());
        service.wait_for(&project, "api", ProcessState::Running);
        let ServiceResponse::Batch(again) = service.send(ServiceRequest::Up {
            project: project.clone(),
        }) else {
            panic!("up answers what it did");
        };
        assert!(again.is_empty(), "a second up starts nothing: {again:?}");

        let ServiceResponse::Log(log) = service.send(ServiceRequest::Log {
            project: project.clone(),
            name: ProcessName::try_new("worker").unwrap(),
        }) else {
            panic!("a stopped process's log is found");
        };
        assert_eq!(log.file_name().unwrap(), "worker.log");

        let ServiceResponse::Batch(down) = service.send(ServiceRequest::Down {
            project: project.clone(),
            agents: false,
        }) else {
            panic!("down answers what it did");
        };
        assert_eq!(down.acted(), &vec![ProcessName::try_new("api").unwrap()]);
        service.stop();
    }

    /// A launch result whose project has no supervisor was never recorded as
    /// one a shutdown detaches, so its process is discarded, never left
    /// running unsupervised.
    #[test]
    fn a_result_for_a_project_without_a_supervisor_is_discarded() {
        let kills = Arc::new(AtomicUsize::new(0));

        release_unrecorded(Ok(Box::new(KillCountHandle {
            kills: Arc::clone(&kills),
        })));

        assert_eq!(kills.load(Ordering::SeqCst), 1);
    }
}
