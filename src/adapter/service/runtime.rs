use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, PoisonError},
    thread,
    time::{Duration, Instant, SystemTime},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};

use super::{
    acceptor::Acceptor,
    event::ServiceEvent,
    event_log::{EventDraft, EventLog, SharedEventLog},
    holder_pool::HolderPool,
    launch_environment::LaunchEnvironment,
    located::Located,
    notice::Notice,
    notice_level::NoticeLevel,
    notices::{Notices, SharedNotices},
    port_sampler::{PORT_SAMPLE_INTERVAL, PortSampler},
    probe_worker::ProbeWorker,
    process_action::ProcessAction,
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
    stack_event::{EventKind, EventSeq},
    supervisor::{HolderScreen, ScreenReply, StaleRelease, Supervisor, SupervisorDeps},
};
use crate::{
    adapter::{bridge::WorkspaceRequest, holder::HolderRecord},
    domain::{
        config::{ConfigError, Stack},
        port::{ProcessHandle, SettingsStore, StackStore},
        project::Project,
        pty::PtyError,
        settings::Settings,
        value::{ProcessName, ProjectName, ProjectRoot},
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
/// Shown when the process has no running child.
const NOT_RUNNING: &str = "the process is not running";
/// Shown when a running process has no holder record to reach it by.
const NO_HOLDER: &str = "the process has no holder to reach it by";
/// Prefix of the refusal of a request the stored state could not answer.
const STORE_UNREADABLE: &str = "could not read the stored state";
/// Prefix of the notice for a holder record that cannot be read.
const UNREADABLE_RECORD_NOTICE: &str = "could not read the holder record";
/// What an unreadable record means for its process.
const UNREADABLE_RECORD_OUTCOME: &str =
    "its process, if it still runs, is left running unsupervised";
/// Prefix of the notice for a project whose running processes could not be
/// adopted because its stack could not be read.
const UNADOPTABLE_PROJECT_NOTICE: &str = "processes of this project are running but its stack cannot be read; they are left running until it can";
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
    /// By a wait that holds the reply and answers it when it ends.
    Held,
    /// Once the service has exited.
    Exit,
}

/// A client waiting for a project's next lifecycle event.
struct EventWaiter {
    /// The project whose events it reads.
    project: ProjectRoot,
    /// Events after this one count.
    since: EventSeq,
    /// The kinds it reads, every kind when empty.
    kinds: Vec<EventKind>,
    /// When it is answered with no events.
    deadline: Instant,
    /// Where the answer goes.
    reply: Sender<ServiceResponse>,
}

/// The one service of a user: owns every loaded project's supervisor on a
/// single thread, which every other thread only sends events to. Clients
/// connect on `service.sock`; holders run every process and outlive it. The
/// service only reads the stored stacks and settings; writers tell it what
/// changed.
pub struct ServiceRuntime {
    /// The running binary's version.
    version: String,
    /// The running binary's path.
    binary: PathBuf,
    /// The holders every process runs under.
    holders: Arc<dyn HolderPool>,
    /// Builds each project's driven adapters.
    ports: Box<dyn ProjectPorts>,
    /// Reads each project's stack.
    stacks: Box<dyn StackStore>,
    /// Reads the user's settings.
    settings_store: Box<dyn SettingsStore>,
    /// The settings every supervisor launches with, as last read.
    settings: Settings,
    /// Where processes, timers, workers and clients deliver events.
    events: Sender<ServiceEvent>,
    /// Notices of every project and of the service.
    notices: SharedNotices,
    /// The supervisor of every loaded project.
    supervisors: HashMap<ProjectRoot, Supervisor>,
    /// Loaded projects that are no longer registered, kept only until the
    /// processes they still ran have stopped.
    unregistered: HashSet<ProjectRoot>,
    /// Lifecycle events of every project and of the service.
    event_log: SharedEventLog,
    /// Clients waiting for a project's next event.
    event_waiters: Vec<EventWaiter>,
    /// Tries port and HTTP probes off the loop.
    probes: ProbeWorker,
    /// Samples the ports running processes listen on, when wired.
    port_sampler: Option<PortSampler>,
    /// When ports are next sampled.
    ports_deadline: Instant,
}

impl ServiceRuntime {
    /// Runs the service until a client asks it to stop or be replaced: reads
    /// the settings, binds the socket in the singleton's state directory,
    /// adopts every running holder into its project's supervisor, and serves.
    /// On exit every holder keeps running, the socket is removed and the lock
    /// released before the exiting client is answered.
    ///
    /// # Errors
    /// Returns a [`ServiceError`] when the settings cannot be read or the
    /// socket cannot be bound.
    pub fn run(singleton: Singleton, deps: ServiceDeps) -> Result<(), ServiceError> {
        let state_dir = singleton.state_dir().clone();
        let settings = deps.settings.load().map_err(ServiceError::Settings)?;
        let (sender, events) = bounded(EVENT_CAPACITY);
        let mut acceptor = Acceptor::bind(&protocol::socket_path(&state_dir), sender.clone())?;
        let mut runtime = Self::new(deps, settings, sender, &state_dir);
        runtime.adopt_running();
        let exit = runtime.serve(&events);
        runtime.shutdown();
        acceptor.close();
        runtime.detach_launches(&events);
        runtime.supervisors.clear();
        runtime.event_waiters.clear();
        drop(events);
        drop(singleton);
        if let Some(reply) = exit {
            let _ = reply.send(ServiceResponse::Ok);
        }
        acceptor.drain();
        Ok(())
    }

    /// A runtime over `deps` launching with `settings`, with no project
    /// loaded, mirroring its notices to the log in `state_dir`.
    fn new(
        deps: ServiceDeps,
        settings: Settings,
        events: Sender<ServiceEvent>,
        state_dir: &Path,
    ) -> Self {
        let ServiceDeps {
            version,
            binary,
            holders,
            ports,
            stacks,
            settings: settings_store,
            listening,
        } = deps;
        Self {
            version,
            binary,
            holders,
            ports,
            stacks,
            settings_store,
            settings,
            probes: ProbeWorker::spawn(events.clone()),
            port_sampler: listening.map(|listening| PortSampler::spawn(listening, events.clone())),
            ports_deadline: Instant::now(),
            events,
            notices: Notices::shared(Some(protocol::log_path(state_dir))),
            supervisors: HashMap::new(),
            unregistered: HashSet::new(),
            event_log: EventLog::shared(),
            event_waiters: Vec::new(),
        }
    }

    /// Takes over what a previous service left: removes the per-holder logs
    /// of an earlier muster and stale handover files, reports every holder
    /// record that cannot be read, and loads each project with running
    /// holders so they are adopted. A project no longer registered is loaded
    /// over an empty stack, so what it still runs is stopped. A project whose
    /// stack cannot be read is reported, and its holders are adopted once it
    /// can be.
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
        let projects: BTreeSet<ProjectRoot> = records
            .iter()
            .map(|record| record.project().clone())
            .collect();
        for project in projects {
            if let Err(refused) = self.load(&project, &records) {
                self.notify(
                    Some(project),
                    NoticeLevel::Error,
                    format!("{UNADOPTABLE_PROJECT_NOTICE}: {}", refused.message()),
                );
            }
        }
    }

    /// Handles events until a client asks the service to exit, waking for the
    /// supervisors' metrics deadlines in between. Returns where to answer
    /// that client.
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

    /// The earliest deadline of any supervisor's metrics, waits or readiness
    /// checks, of any client waiting for events, and of the next port sample.
    fn next_deadline(&self) -> Option<Instant> {
        let supervisors = self.supervisors.values().flat_map(|supervisor| {
            [
                supervisor.next_metrics_deadline(),
                supervisor.next_wait_deadline(),
            ]
        });
        let events = self
            .event_waiters
            .iter()
            .map(|waiter| Some(waiter.deadline));
        let ports = self
            .port_sampler
            .as_ref()
            .filter(|_| !self.live_root_pids().is_empty())
            .map(|_| self.ports_deadline);
        supervisors.chain(events).chain([ports]).flatten().min()
    }

    /// Acknowledges the held exits whose restart settled, samples usage and
    /// ports where due, settles waits, answers clients waiting for events,
    /// and lets go of an unregistered project once it runs nothing.
    fn tick(&mut self, now: Instant) {
        for supervisor in self.supervisors.values_mut() {
            supervisor.release_settled_exits();
            supervisor.sample_metrics(now);
            supervisor.settle_waits(now);
        }
        self.sample_ports(now);
        self.answer_event_waiters(now);
        let finished: Vec<ProjectRoot> = self
            .unregistered
            .iter()
            .filter(|root| self.supervisors.get(*root).is_none_or(Supervisor::is_idle))
            .cloned()
            .collect();
        for root in finished {
            self.unregistered.remove(&root);
            self.supervisors.remove(&root);
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
            ServiceEvent::ProbePassed {
                project,
                pane,
                generation,
                probe,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_probe_passed(pane, generation, &probe);
                }
            },
            ServiceEvent::PortsSampled { ports } => {
                for supervisor in self.supervisors.values_mut() {
                    supervisor.take_ports(&ports);
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
        let response = match self.respond(request, &reply) {
            Ok(Reply::Now(response)) => response,
            Ok(Reply::Screen(ScreenReply::Ready(screen))) => ServiceResponse::Screen(screen),
            Ok(Reply::Screen(ScreenReply::ReadHolder(screen))) => {
                self.read_off_loop(screen, reply);
                return Flow::Continue;
            },
            Ok(Reply::Held) => return Flow::Continue,
            Ok(Reply::Exit) => return Flow::Exit(reply),
            Err(refusal) => ServiceResponse::Error(refusal),
        };
        let _ = reply.send(response);
        Flow::Continue
    }

    /// Carries out one client request; a wait keeps a clone of `reply` to
    /// answer on when it ends.
    ///
    /// # Errors
    /// Returns the refusal to send when the project or process is unknown, a
    /// wait cannot be taken, or the stored state cannot be read.
    fn respond(
        &mut self,
        request: ServiceRequest,
        reply: &Sender<ServiceResponse>,
    ) -> Result<Reply, ServiceErrorBody> {
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
            ServiceRequest::Hello { .. } => ServiceResponse::Hello {
                version: self.version.clone(),
                binary: self.binary.clone(),
            },
            ServiceRequest::Up {
                project,
                environment,
                wait: None,
            } => ServiceResponse::Batch(self.supervisor(&project)?.up(&environment)),
            ServiceRequest::Up {
                project,
                environment,
                wait: Some(wait),
            } => {
                self.supervisor(&project)?
                    .up_waiting(&environment, &wait, reply.clone())?;
                return Ok(Reply::Held);
            },
            ServiceRequest::Wait {
                project,
                name,
                wait,
            } => {
                let supervisor = self.process(&project, &name)?;
                let prepared = supervisor.prepare_wait(&name, &wait)?;
                supervisor.begin_wait(prepared, None, reply.clone());
                return Ok(Reply::Held);
            },
            ServiceRequest::Events {
                project,
                since,
                kinds,
                wait,
            } => return self.events_after(project, since, kinds, wait, reply),
            ServiceRequest::Down { project } => {
                ServiceResponse::Batch(self.supervisor(&project)?.down())
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
                environment,
                wait: None,
            } => {
                control(self.process(&project, &name)?, &name, action, &environment);
                ServiceResponse::Ok
            },
            ServiceRequest::Control {
                project,
                name,
                action,
                environment,
                wait: Some(wait),
            } => {
                let supervisor = self.process(&project, &name)?;
                let prepared = supervisor.prepare_wait(&name, &wait)?;
                let fresh = match action {
                    ProcessAction::Start => !supervisor.runs_or_starts(&name),
                    ProcessAction::Restart => true,
                    ProcessAction::Stop
                    | ProcessAction::Pause
                    | ProcessAction::Resume
                    | ProcessAction::Kill => false,
                };
                let before = supervisor.generation_of(&name);
                control(supervisor, &name, action, &environment);
                supervisor.begin_wait(prepared, fresh.then_some(before), reply.clone());
                return Ok(Reply::Held);
            },
            ServiceRequest::Locate { project, name } => {
                ServiceResponse::Located(self.locate(&project, &name)?)
            },
            ServiceRequest::Notices { project } => ServiceResponse::Notices(
                self.notices
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .recent(&project),
            ),
            ServiceRequest::StackChanged { project } => {
                self.stack_changed(&project)?;
                ServiceResponse::Ok
            },
            ServiceRequest::SettingsChanged => {
                self.settings_changed()?;
                ServiceResponse::Ok
            },
        };
        Ok(Reply::Now(response))
    }

    /// Applies `project`'s stored stack to its supervisor when it is loaded;
    /// one no longer registered applies an empty stack, so what it runs is
    /// stopped. An unloaded project reads its stack afresh when it loads.
    ///
    /// # Errors
    /// Returns the refusal when the stack cannot be read.
    fn stack_changed(&mut self, project: &ProjectRoot) -> Result<(), ServiceErrorBody> {
        if !self.supervisors.contains_key(project) {
            return Ok(());
        }
        let stored = self.stacks.stack(project).map_err(store_refusal)?;
        let Some(supervisor) = self.supervisors.get_mut(project) else {
            return Ok(());
        };
        let stack = match stored {
            Some(stack) => {
                self.unregistered.remove(project);
                stack
            },
            None => {
                self.unregistered.insert(project.clone());
                empty_stack(supervisor.project().clone())
            },
        };
        supervisor.apply_stack(&stack);
        Ok(())
    }

    /// Reads the stored settings for every later launch, and hands them to
    /// every supervisor.
    ///
    /// # Errors
    /// Returns the refusal when the settings cannot be read.
    fn settings_changed(&mut self) -> Result<(), ServiceErrorBody> {
        self.settings = self.settings_store.load().map_err(store_refusal)?;
        for supervisor in self.supervisors.values_mut() {
            supervisor.apply_settings(&self.settings);
        }
        Ok(())
    }

    /// Every loaded project's status, ordered by project folder.
    fn statuses(&self) -> Vec<ProjectStatus> {
        let mut statuses: Vec<_> = self
            .supervisors
            .values()
            .map(Supervisor::snapshot)
            .collect();
        statuses.sort_by(|left, right| left.project().root().cmp(right.project().root()));
        statuses
    }

    /// Where the running process `name` of `project` can be reached.
    ///
    /// # Errors
    /// Returns the refusal when the process is unknown, not running, or has
    /// no holder record.
    fn locate(
        &mut self,
        project: &ProjectRoot,
        name: &ProcessName,
    ) -> Result<Located, ServiceErrorBody> {
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
        project: &ProjectRoot,
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

    /// The supervisor of `project`, loading it on first use from its stored
    /// stack and adopting the holders already running its processes.
    ///
    /// # Errors
    /// Returns the refusal when `project` is not registered or its stack
    /// cannot be read.
    fn supervisor(&mut self, project: &ProjectRoot) -> Result<&mut Supervisor, ServiceErrorBody> {
        if !self.supervisors.contains_key(project) {
            let records = self.holders.live_records();
            self.load(project, &records)?;
        }
        self.supervisors
            .get_mut(project)
            .ok_or_else(|| unknown_project(project))
    }

    /// Loads `project`'s supervisor over its stored stack and adopts each of
    /// `records` that runs one of its processes. A project no longer
    /// registered is loaded over an empty stack only while `records` show it
    /// still runs something, which is then stopped.
    ///
    /// # Errors
    /// Returns the refusal when `project` is not registered and runs nothing,
    /// or its stack cannot be read.
    fn load(
        &mut self,
        project: &ProjectRoot,
        records: &[HolderRecord],
    ) -> Result<(), ServiceErrorBody> {
        let held: Vec<&HolderRecord> = records
            .iter()
            .filter(|record| record.project() == project)
            .collect();
        let stack = match self.stacks.stack(project).map_err(store_refusal)? {
            Some(stack) => stack,
            None if !held.is_empty() => {
                let name = ProjectName::candidates(project)
                    .next()
                    .ok_or_else(|| unknown_project(project))?;
                self.unregistered.insert(project.clone());
                empty_stack(Project::builder().root(project.clone()).name(name).build())
            },
            None => return Err(unknown_project(project)),
        };
        let shared = SharedHolders::new(Arc::clone(&self.holders));
        let deps = SupervisorDeps::builder()
            .runner(Box::new(shared.clone()))
            .events(self.events.clone())
            .notices(Arc::clone(&self.notices))
            .event_log(Arc::clone(&self.event_log))
            .probes(Some(self.probes.clone()))
            .notifier(self.ports.notifier())
            .process_metrics(self.ports.process_metrics())
            .settings(self.settings.clone())
            .build();
        let mut supervisor =
            Supervisor::new(stack.project().clone(), &stack, deps).with_adopter(Box::new(shared));
        for record in held {
            supervisor.adopt(record.clone());
        }
        self.supervisors.insert(project.clone(), supervisor);
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

    /// Answers a read of `project`'s events after `since` of `kinds`: at once
    /// when there are any or `wait` is zero, otherwise when the next one
    /// arrives or `wait` passes.
    ///
    /// # Errors
    /// Returns the refusal when the project cannot be loaded.
    fn events_after(
        &mut self,
        project: ProjectRoot,
        since: Option<EventSeq>,
        kinds: Vec<EventKind>,
        wait: Duration,
        reply: &Sender<ServiceResponse>,
    ) -> Result<Reply, ServiceErrorBody> {
        self.supervisor(&project)?;
        let log = self
            .event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let page = log.page(&project, since, &kinds);
        if !page.events().is_empty() || wait.is_zero() {
            return Ok(Reply::Now(ServiceResponse::Events(page)));
        }
        let since = since.unwrap_or_else(|| log.latest());
        drop(log);
        self.event_waiters.push(EventWaiter {
            project,
            since,
            kinds,
            deadline: Instant::now() + wait,
            reply: reply.clone(),
        });
        Ok(Reply::Held)
    }

    /// Answers each client waiting for events once one arrived, or with none
    /// once its wait passed.
    fn answer_event_waiters(&mut self, now: Instant) {
        if self.event_waiters.is_empty() {
            return;
        }
        let log = self
            .event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.event_waiters.retain(|waiter| {
            let page = log.page(&waiter.project, Some(waiter.since), &waiter.kinds);
            if page.events().is_empty() && now < waiter.deadline {
                return true;
            }
            let _ = waiter.reply.send(ServiceResponse::Events(page));
            false
        });
    }

    /// Asks the port sampler for the ports of every live process once the
    /// sample interval passed.
    fn sample_ports(&mut self, now: Instant) {
        let Some(sampler) = &self.port_sampler else {
            return;
        };
        if now < self.ports_deadline {
            return;
        }
        let roots = self.live_root_pids();
        if roots.is_empty() {
            return;
        }
        self.ports_deadline = now + PORT_SAMPLE_INTERVAL;
        sampler.sample(roots);
    }

    /// The pid of every live process of every loaded project.
    fn live_root_pids(&self) -> Vec<u32> {
        self.supervisors
            .values()
            .flat_map(Supervisor::live_root_pids)
            .collect()
    }

    /// Records a notice about `project`, or the service as a whole.
    fn notify(&self, project: Option<ProjectRoot>, level: NoticeLevel, message: String) {
        self.event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(
                EventDraft::builder()
                    .project(project.clone())
                    .kind(EventKind::Notice)
                    .detail(message.clone())
                    .build(),
            );
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

/// Applies `action` to the process `name` of `supervisor`; a start or restart
/// runs with `environment`, the environment of the client that asked.
fn control(
    supervisor: &mut Supervisor,
    name: &ProcessName,
    action: ProcessAction,
    environment: &LaunchEnvironment,
) {
    match action {
        ProcessAction::Start => supervisor.start(name, environment),
        ProcessAction::Stop => supervisor.stop(name),
        ProcessAction::Restart => supervisor.restart(name, environment),
        ProcessAction::Pause => supervisor.pause(name),
        ProcessAction::Resume => supervisor.resume(name),
        ProcessAction::Kill => supervisor.force_kill(name),
    }
}

/// The stack of `project` with no process in it.
fn empty_stack(project: Project) -> Stack {
    Stack::builder()
        .project(project)
        .processes(Vec::new())
        .build()
}

/// A refusal of `kind` saying `message`.
fn refused(kind: ServiceErrorKind, message: String) -> ServiceErrorBody {
    ServiceErrorBody::builder()
        .kind(kind)
        .message(message)
        .build()
}

/// The refusal of `project`, which is not registered.
fn unknown_project(project: &ProjectRoot) -> ServiceErrorBody {
    refused(
        ServiceErrorKind::UnknownProject,
        ConfigError::UnknownProject {
            root: project.as_ref().clone(),
        }
        .to_string(),
    )
}

/// The refusal of a request the stored state failed to answer with `error`.
fn store_refusal(error: ConfigError) -> ServiceErrorBody {
    refused(
        ServiceErrorKind::Internal,
        format!("{STORE_UNREADABLE}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        os::unix::net::{UnixListener, UnixStream},
        process,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread::JoinHandle,
    };

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            holder::{
                ClientMessage, ClientRole, HolderConfig, HolderDirs, HolderError, HolderId,
                HolderLauncher, HolderRunner, HolderServer, LogQuery, abandon, client::greet,
                read_log, testing::test_request,
            },
            ipc,
            service::{
                BatchOutcome, ProcessStatus, StateGoal, WaitCondition, WaitSpec, WaitStatus,
                supervisor::{
                    ChannelOutputSink, HolderAdopter, SpawnGeneration,
                    testing::{KillCountHandle, Orphan, test_settings, test_stop},
                },
            },
        },
        domain::{
            config::{
                Environment, ImportMode, ImportPlan, ProcessChange, ProcessSpec, Registration,
                SettingChange,
            },
            port::{Notifier, OutputSink, ProcessHandle, ProcessMetrics, ProcessRunner},
            process::{LogPattern, PatternKind, PatternText, ProcessState, RestartPolicy},
            pty::{ProcessOutput, PtySize, SpawnRequest},
            value::{Cols, CommandLine, PaneId, Rows},
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

    /// Stacks and settings held in memory and shared with the test, which
    /// edits them as a writer would before telling the service.
    #[derive(Clone)]
    struct MemoryStore {
        stacks: Arc<Mutex<HashMap<ProjectRoot, Vec<ProcessSpec>>>>,
        settings: Arc<Mutex<Settings>>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                stacks: Arc::default(),
                settings: Arc::new(Mutex::new(test_settings())),
            }
        }

        /// Registers `root` with `processes`, replacing its stack.
        fn put(&self, root: &ProjectRoot, processes: Vec<ProcessSpec>) {
            self.stacks.lock().unwrap().insert(root.clone(), processes);
        }

        /// Unregisters `root`.
        fn forget(&self, root: &ProjectRoot) {
            self.stacks.lock().unwrap().remove(root);
        }

        /// The project at `root`, named for its folder.
        fn project(root: &ProjectRoot) -> Project {
            Project::builder()
                .root(root.clone())
                .name(ProjectName::candidates(root).next().unwrap())
                .build()
        }
    }

    impl StackStore for MemoryStore {
        fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
            Ok(self.stacks.lock().unwrap().get(root).map(|processes| {
                Stack::builder()
                    .project(Self::project(root))
                    .processes(processes.clone())
                    .build()
            }))
        }

        fn add_process(
            &self,
            root: &ProjectRoot,
            spec: &ProcessSpec,
        ) -> Result<Registration, ConfigError> {
            let mut stacks = self.stacks.lock().unwrap();
            let newly_registered = !stacks.contains_key(root);
            stacks.entry(root.clone()).or_default().push(spec.clone());
            Ok(Registration::builder()
                .project(Self::project(root))
                .newly_registered(newly_registered)
                .build())
        }

        fn change_process(
            &self,
            root: &ProjectRoot,
            name: &ProcessName,
            change: &ProcessChange,
        ) -> Result<ProcessSpec, ConfigError> {
            let mut stacks = self.stacks.lock().unwrap();
            let spec = stacks
                .get_mut(root)
                .and_then(|processes| processes.iter_mut().find(|spec| spec.name() == name))
                .ok_or_else(|| ConfigError::Database(name.to_string()))?;
            *spec = spec.clone().with_change(change);
            Ok(spec.clone())
        }

        fn remove_process(
            &self,
            root: &ProjectRoot,
            name: &ProcessName,
        ) -> Result<ProcessSpec, ConfigError> {
            let mut stacks = self.stacks.lock().unwrap();
            let processes = stacks
                .get_mut(root)
                .ok_or_else(|| ConfigError::Database(name.to_string()))?;
            let index = processes
                .iter()
                .position(|spec| spec.name() == name)
                .ok_or_else(|| ConfigError::Database(name.to_string()))?;
            Ok(processes.remove(index))
        }

        fn import(
            &self,
            _root: &ProjectRoot,
            _processes: &[ProcessSpec],
            _mode: ImportMode,
        ) -> Result<(Registration, ImportPlan), ConfigError> {
            Err(ConfigError::Database("not used by these tests".to_string()))
        }
    }

    impl SettingsStore for MemoryStore {
        fn load(&self) -> Result<Settings, ConfigError> {
            Ok(self.settings.lock().unwrap().clone())
        }

        fn apply(&self, change: &SettingChange) -> Result<Settings, ConfigError> {
            let mut settings = self.settings.lock().unwrap();
            *settings = settings.clone().with_change(change)?;
            Ok(settings.clone())
        }
    }

    /// Ports with nothing wired.
    struct TestPorts;

    impl ProjectPorts for TestPorts {
        fn notifier(&self) -> Option<Box<dyn Notifier>> {
            None
        }

        fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
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

        fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
            project.as_ref().join(format!("{name}.log"))
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

    /// Polls `condition` until it holds, failing the test at [`WAIT`].
    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
            thread::sleep(POLL);
        }
    }

    /// A process `name` running [`SLEEPER`], starting with the project when
    /// `autostart`.
    fn sleeper(name: &str, autostart: bool) -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(SLEEPER).unwrap())
            .environment(Environment::empty())
            .autostart(autostart)
            .restart(RestartPolicy::Never)
            .stop(test_stop())
            .build()
    }

    /// Processes `(name, autostart)`, each running [`SLEEPER`].
    fn commands(entries: &[(&str, bool)]) -> Vec<ProcessSpec> {
        entries
            .iter()
            .map(|(name, autostart)| sleeper(name, *autostart))
            .collect()
    }

    /// A client asking with no environment of its own.
    fn no_env() -> LaunchEnvironment {
        LaunchEnvironment::default()
    }

    /// A scratch state directory, holder dirs, projects and their store, plus
    /// every holder thread started there; dropping it kills every child still
    /// recorded and lets its holder finish.
    struct Fixture {
        scratch: TempDir,
        holders: Holders,
        store: MemoryStore,
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
                store: MemoryStore::new(),
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
            HolderRunner::new(
                Box::new(ThreadLauncher(Arc::clone(&self.holders))),
                self.dirs(),
            )
        }

        /// As [`Self::runner`], whose holders start only once `gate` opens.
        fn gated_runner(&self, gate: Receiver<()>) -> HolderRunner {
            HolderRunner::new(
                Box::new(GatedLauncher {
                    holders: Arc::clone(&self.holders),
                    gate,
                }),
                self.dirs(),
            )
        }

        /// The folder `folder` in the scratch directory, created.
        fn folder(&self, folder: &str) -> ProjectRoot {
            let dir = self.scratch.path().join(folder);
            fs::create_dir_all(&dir).unwrap();
            ProjectRoot::try_new(fs::canonicalize(dir).unwrap()).unwrap()
        }

        /// Registers the project `project` with `processes` and returns its
        /// folder.
        fn project(&self, processes: Vec<ProcessSpec>) -> ProjectRoot {
            self.project_in("project", processes)
        }

        /// Registers the project in `folder` with `processes` and returns it.
        fn project_in(&self, folder: &str, processes: Vec<ProcessSpec>) -> ProjectRoot {
            let root = self.folder(folder);
            self.store.put(&root, processes);
            root
        }

        /// A request for `process` running `command` in `project`.
        fn request(&self, project: &ProjectRoot, process: &str, command: &str) -> SpawnRequest {
            test_request(
                process,
                command,
                Some(project.as_ref().clone()),
                Some(self.scratch.path().to_path_buf()),
            )
        }

        /// Starts a service over `pool` and waits until it accepts clients.
        fn start(&self, pool: Arc<dyn HolderPool>) -> Running {
            let state_dir = self.state_dir();
            let socket = protocol::socket_path(&state_dir);
            let store = self.store.clone();
            let thread = thread::spawn(move || {
                let Some(singleton) = Singleton::acquire(&state_dir)? else {
                    return Ok(());
                };
                let deps = ServiceDeps::builder()
                    .version(VERSION.to_string())
                    .binary(PathBuf::from(TEST_BINARY))
                    .holders(pool)
                    .ports(Box::new(TestPorts))
                    .stacks(Box::new(store.clone()))
                    .settings(Box::new(store))
                    .listening(None)
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

        /// Brings `project` up with no client environment.
        fn up(&self, project: &ProjectRoot) -> ServiceResponse {
            self.send(ServiceRequest::Up {
                project: project.clone(),
                environment: no_env(),
                wait: None,
            })
        }

        /// Applies `action` to `name` of `project` with no client environment.
        fn control(
            &self,
            project: &ProjectRoot,
            name: &str,
            action: ProcessAction,
        ) -> ServiceResponse {
            self.send(ServiceRequest::Control {
                project: project.clone(),
                name: ProcessName::try_new(name).unwrap(),
                action,
                environment: no_env(),
                wait: None,
            })
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
        fn wait_for(
            &self,
            project: &ProjectRoot,
            name: &str,
            state: ProcessState,
        ) -> ProjectStatus {
            let mut status = self.status(project);
            wait_until("the process to reach its state", || {
                status = self.status(project);
                find(&status, name).is_some_and(|process| *process.state() == state)
            });
            status
        }

        /// The one project status a `ps` of `project` reports.
        fn status(&self, project: &ProjectRoot) -> ProjectStatus {
            let ServiceResponse::Status(mut statuses) = self.send(ServiceRequest::Ps {
                project: Some(project.clone()),
            }) else {
                panic!("ps answers a status");
            };
            statuses.remove(0)
        }

        /// The notices of `project`.
        fn notices(&self, project: &ProjectRoot) -> Vec<Notice> {
            let ServiceResponse::Notices(notices) = self.send(ServiceRequest::Notices {
                project: project.clone(),
            }) else {
                panic!("notices answer notices");
            };
            notices
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

    /// The status of the process `name` in `status`, if it has a row.
    fn find<'a>(status: &'a ProjectStatus, name: &str) -> Option<&'a ProcessStatus> {
        status
            .processes()
            .iter()
            .find(|process| process.name().as_ref() == name)
    }

    /// The status of the process `name` in `status`.
    fn process<'a>(status: &'a ProjectStatus, name: &str) -> &'a ProcessStatus {
        find(status, name).unwrap()
    }

    #[test]
    fn service_adopts_live_holders_on_start_and_ps_reports_them_running() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("dev server", false)]));
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
        assert_eq!(status.project().root(), &project);
        service.stop();
    }

    #[test]
    fn stop_request_exits_the_loop_and_holders_keep_running() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
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
        assert_eq!(*process(&api, "api").pid(), Some(pid), "the same child");
        successor.stop();
    }

    #[test]
    fn unknown_process_error_lists_candidates() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("dev server", false), ("café", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));

        let response = service.control(&project, "nope", ProcessAction::Start);

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
        let project = fixture.project(commands(&[("eager", true), ("lazy", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        assert_eq!(
            service.send(ServiceRequest::Ps { project: None }),
            ServiceResponse::Status(Vec::new()),
            "nothing is loaded before it is asked for"
        );

        assert_eq!(
            service.up(&project),
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

    /// A folder that is no registered project is refused as unknown, saying
    /// how to register one.
    #[test]
    fn an_unregistered_project_is_refused() {
        let fixture = Fixture::new();
        let service = fixture.start(Arc::new(fixture.runner()));

        let ServiceResponse::Error(missing) = service.up(&fixture.folder("absent")) else {
            panic!("an unregistered project is refused");
        };
        assert_eq!(*missing.kind(), ServiceErrorKind::UnknownProject);
        assert!(
            missing.message().contains("muster add"),
            "{}",
            missing.message()
        );
        service.stop();
    }

    /// Input is answered with `Ok` once delivered, and with the standard
    /// refusals for a stopped process and an unknown name, which lists the
    /// valid ones.
    #[test]
    fn input_is_delivered_or_refused_like_any_process_request() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", false)]));
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

        service.control(&project, "api", ProcessAction::Start);
        service.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(send("api"), ServiceResponse::Ok);
        service.stop();
    }

    /// `Locate` names the holder socket and log of a running process, and a
    /// writable client attaching there is counted as the holder reports it,
    /// down again once its connection closes.
    #[test]
    fn locate_names_the_holder_and_attached_is_counted() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
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

        service.control(&project, "api", ProcessAction::Kill);
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
        let project = fixture.project(commands(&[("api", true)]));
        let stalled = fixture.scratch.path().join("stalled.sock");
        let _never_accepts = UnixListener::bind(&stalled).unwrap();
        let pool = FakePool::new(Vec::new(), Some(stalled));
        let service = fixture.start(Arc::new(pool));
        service.up(&project);

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
        let project = fixture.project(commands(&[("api", false)]));
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
        let notices = service.notices(&project);
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
        let project = fixture.project(commands(&[("api", false)]));
        let pool = FakePool::new(Vec::new(), None);
        *pool.legacy_result.lock().unwrap() = Some(io::Error::other("read-only file system"));
        let removals = Arc::clone(&pool.legacy_removals);

        let service = fixture.start(Arc::new(pool));

        assert_eq!(removals.load(Ordering::SeqCst), 1);
        let notices = service.notices(&project);
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

    /// The record a holder running `name` of `project` as `child_pid` would
    /// leave, its socket gone.
    fn record(
        fixture: &Fixture,
        project: &ProjectRoot,
        name: &str,
        child_pid: u32,
    ) -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(project.clone())
            .process(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(SLEEPER).unwrap())
            .stop(test_stop())
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
        let project = fixture.project(commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let pool = FakePool::new(vec![record(&fixture, &project, "api", orphan.pid())], None);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        let name = ProcessName::try_new("api").unwrap();

        service.wait_for(&project, "api", ProcessState::Unsupervised);
        let ServiceResponse::Batch(up) = service.up(&project) else {
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
        service.control(&project, "api", ProcessAction::Start);

        let status = service.status(&project);
        assert_eq!(*process(&status, "api").state(), ProcessState::Unsupervised);
        assert_eq!(*process(&status, "api").pid(), Some(orphan.pid()));
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");
        assert!(
            service
                .notices(&project)
                .iter()
                .any(|notice| notice.message().ends_with(&orphan.pid().to_string())),
            "the refused start names the pid"
        );
        service.stop();
    }

    /// A launch whose holder never becomes ready runs off the loop: the
    /// process shows `starting`, and a request for another project is
    /// answered at once meanwhile.
    #[test]
    fn a_holder_that_never_becomes_ready_does_not_delay_another_project() {
        let fixture = Fixture::new();
        let slow = fixture.project_in("slow", commands(&[("api", true)]));
        let other = fixture.project_in("other", commands(&[("web", false)]));
        let (open, gate) = bounded(0);
        let service = fixture.start(Arc::new(FakePool::new(Vec::new(), None).gated(gate)));

        service.up(&slow);
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
            let project = fixture.project(commands(&[("api", true)]));
            let (open, gate) = bounded(0);
            let pool = FakePool::new(Vec::new(), None).gated(gate);
            let kills = Arc::clone(&pool.kills);
            let service = fixture.start(Arc::new(pool));
            service.up(&project);
            service.wait_for(&project, "api", ProcessState::Starting);

            service.control(&project, "api", action);
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
        let project = fixture.project(commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let (open, gate) = bounded(0);
        let pool =
            FakePool::new(vec![record(&fixture, &project, "api", orphan.pid())], None).gated(gate);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        service.wait_for(&project, "api", ProcessState::Starting);

        for action in [ProcessAction::Stop, ProcessAction::Start] {
            service.control(&project, "api", action);
        }
        let ServiceResponse::Ok = service.control(&project, "api", ProcessAction::Stop) else {
            panic!("the stop is accepted while starting");
        };
        drop(open);

        service.wait_for(&project, "api", ProcessState::Stopping);
        service.control(&project, "api", ProcessAction::Start);
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
        let project = fixture.project(commands(&[("api", true)]));
        let (open, gate) = bounded(0);
        let service = fixture.start(Arc::new(fixture.gated_runner(gate)));
        service.up(&project);
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
        let project = fixture.project(commands(&[("api", true)]));
        let (open, gate) = bounded(0);
        let pool = FakePool::new(Vec::new(), None).gated(gate);
        let spawns = Arc::clone(&pool.spawns);
        let service = fixture.start(Arc::new(pool));
        service.up(&project);
        service.wait_for(&project, "api", ProcessState::Starting);

        service.control(&project, "api", ProcessAction::Start);
        service.up(&project);
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
        let project = fixture.project(commands(&[("api", true)]));
        let orphan = Orphan::spawn();
        let (open, gate) = bounded(0);
        let pool =
            FakePool::new(vec![record(&fixture, &project, "api", orphan.pid())], None).gated(gate);
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
        let project = fixture.project(commands(&[("flood", false)]));
        let (sender, events) = bounded(EVENT_CAPACITY);
        let sink = ChannelOutputSink::new(
            project.clone(),
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
                    output: ProcessOutput::Repaint(bytes),
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

    /// A project named by a relative path is refused as unreadable, since it
    /// would resolve against the service's directory.
    #[test]
    fn a_relative_project_is_refused() {
        let fixture = Fixture::new();
        let service = fixture.start(Arc::new(fixture.runner()));

        let response: ServiceResponse = ipc::request(
            &service.socket,
            &serde_json::json!({ "ps": { "project": "project" } }),
        )
        .unwrap();

        let ServiceResponse::Error(refusal) = response else {
            panic!("a relative project is refused");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::InvalidRequest);
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

    /// A start with a wait answers once the new run prints the line, through
    /// a real holder; its start is an event, a read after the newest event
    /// waits and answers empty, and a stop waits for the process to stop.
    #[test]
    fn a_start_with_a_wait_answers_once_the_new_run_prints_and_events_follow() {
        let fixture = Fixture::new();
        let printer = sleeper("api", false)
            .with_command(CommandLine::try_new("echo ready on 3000; sleep 600").unwrap());
        let project = fixture.project(vec![printer]);
        let service = fixture.start(Arc::new(fixture.runner()));
        let until = WaitCondition::Log(
            LogPattern::builder()
                .text(PatternText::try_new("ready on").unwrap())
                .kind(PatternKind::Substring)
                .build(),
        );
        let wait = WaitSpec::builder().until(Some(until)).timeout(WAIT).build();

        let ServiceResponse::Waited(started) = service.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new("api").unwrap(),
            action: ProcessAction::Start,
            environment: no_env(),
            wait: Some(wait),
        }) else {
            panic!("a start with a wait answers how the wait ended");
        };
        assert_eq!(started.status(), WaitStatus::Matched);
        assert_eq!(started.matched().as_ref().unwrap().line(), "ready on 3000");

        let ServiceResponse::Events(page) = service.send(ServiceRequest::Events {
            project: project.clone(),
            since: None,
            kinds: vec![EventKind::Started],
            wait: Duration::ZERO,
        }) else {
            panic!("events answer with a page");
        };
        assert_eq!(page.events().len(), 1);
        let ServiceResponse::Events(quiet) = service.send(ServiceRequest::Events {
            project: project.clone(),
            since: Some(*page.cursor()),
            kinds: Vec::new(),
            wait: POLL,
        }) else {
            panic!("events answer with a page");
        };
        assert!(quiet.events().is_empty() && !quiet.gap());

        let stopped = WaitSpec::builder()
            .until(Some(WaitCondition::State(StateGoal::Stopped)))
            .timeout(WAIT)
            .build();
        let ServiceResponse::Waited(stop) = service.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new("api").unwrap(),
            action: ProcessAction::Stop,
            environment: no_env(),
            wait: Some(stopped),
        }) else {
            panic!("a stop with a wait answers how the wait ended");
        };
        assert_eq!(stop.status(), WaitStatus::Matched);
        service.stop();
    }

    /// Cursors are the holder log's offsets: a restarted run begins where the
    /// first run's output ended in the log, its matched line lies inside it,
    /// and reading the log from its start returns only that run.
    #[test]
    fn cursors_are_log_offsets_across_a_restart() {
        let fixture = Fixture::new();
        let printer = sleeper("api", false)
            .with_command(CommandLine::try_new("echo ready in run $$; sleep 600").unwrap());
        let project = fixture.project(vec![printer]);
        let service = fixture.start(Arc::new(fixture.runner()));
        let api = ProcessName::try_new("api").unwrap();
        let until = WaitCondition::Log(
            LogPattern::builder()
                .text(PatternText::try_new("ready in run").unwrap())
                .kind(PatternKind::Substring)
                .build(),
        );
        let control = |action| ServiceRequest::Control {
            project: project.clone(),
            name: api.clone(),
            action,
            environment: no_env(),
            wait: Some(
                WaitSpec::builder()
                    .until(Some(until.clone()))
                    .timeout(WAIT)
                    .build(),
            ),
        };

        let ServiceResponse::Waited(first) = service.send(control(ProcessAction::Start)) else {
            panic!("a start with a wait answers how the wait ended");
        };
        let ServiceResponse::Waited(second) = service.send(control(ProcessAction::Restart)) else {
            panic!("a restart with a wait answers how the wait ended");
        };
        assert_eq!(first.status(), WaitStatus::Matched);
        assert_eq!(second.status(), WaitStatus::Matched);
        let ServiceResponse::Log(log) = service.send(ServiceRequest::Log {
            project: project.clone(),
            name: api.clone(),
        }) else {
            panic!("the log is found");
        };
        let run_start = process(&service.status(&project), "api")
            .run_start()
            .expect("a run start");

        let first_line = first.matched().as_ref().unwrap();
        let second_line = second.matched().as_ref().unwrap();
        assert!(
            run_start > *first_line.at(),
            "{run_start} after {}",
            first_line.at()
        );
        assert!(*second_line.at() >= run_start);
        let slice = read_log(&log, &LogQuery::builder().since(Some(run_start)).build()).unwrap();
        assert_eq!(slice.lines(), &vec![second_line.line().clone()]);
        assert_ne!(first_line.line(), second_line.line());
        service.stop();

        let adopting = fixture.start(Arc::new(fixture.runner()));
        let adopted = adopting.wait_for(&project, "api", ProcessState::Running);
        assert_eq!(
            *process(&adopted, "api").run_start(),
            Some(run_start),
            "an adopted run keeps its log offset"
        );
        adopting.stop();
    }

    /// `Up` and `Down` answer with what they started or stopped and what
    /// they left alone, `Down` stops every process, and the log of a process
    /// is known whether or not it runs.
    #[test]
    fn up_and_down_say_what_they_did_and_logs_are_found_for_stopped_processes() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true), ("web", true), ("worker", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));

        let ServiceResponse::Batch(up) = service.up(&project) else {
            panic!("up answers what it did");
        };
        assert_eq!(up.acted(), &vec![
            ProcessName::try_new("api").unwrap(),
            ProcessName::try_new("web").unwrap()
        ]);
        assert!(up.refused().is_empty());
        service.wait_for(&project, "api", ProcessState::Running);
        service.wait_for(&project, "web", ProcessState::Running);
        let ServiceResponse::Batch(again) = service.up(&project) else {
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
        }) else {
            panic!("down answers what it did");
        };
        assert_eq!(down.acted(), &vec![
            ProcessName::try_new("api").unwrap(),
            ProcessName::try_new("web").unwrap()
        ]);
        service.wait_for(&project, "api", ProcessState::Exited);
        service.wait_for(&project, "web", ProcessState::Exited);
        service.stop();
    }

    /// A stack change reaches a loaded project: an added process waits
    /// `Pending`, a changed running one keeps running unrestarted, and a
    /// removed running one is stopped and its row goes.
    #[test]
    fn a_stack_change_adds_updates_and_stops_without_restarting() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true), ("web", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
        let before = service.wait_for(&project, "api", ProcessState::Running);
        service.wait_for(&project, "web", ProcessState::Running);
        let api_pid = process(&before, "api").pid().unwrap();

        let changed = sleeper("api", true).with_command(CommandLine::try_new("sleep 601").unwrap());
        fixture
            .store
            .put(&project, vec![changed, sleeper("docs", false)]);
        assert_eq!(
            service.send(ServiceRequest::StackChanged {
                project: project.clone()
            }),
            ServiceResponse::Ok
        );

        let status = service.status(&project);
        assert_eq!(*process(&status, "docs").state(), ProcessState::Pending);
        assert_eq!(*process(&status, "api").state(), ProcessState::Running);
        assert_eq!(
            *process(&status, "api").pid(),
            Some(api_pid),
            "not restarted"
        );
        wait_until("the removed process to go", || {
            find(&service.status(&project), "web").is_none()
        });
        assert_eq!(
            fixture.runner().live_records().len(),
            1,
            "web's holder ended"
        );
        service.stop();
    }

    /// A change to a project the service has not loaded does nothing: it
    /// reads its stack fresh when it loads.
    #[test]
    fn a_stack_change_to_an_unloaded_project_is_accepted() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));

        assert_eq!(
            service.send(ServiceRequest::StackChanged {
                project: project.clone()
            }),
            ServiceResponse::Ok
        );
        assert_eq!(
            service.send(ServiceRequest::Ps { project: None }),
            ServiceResponse::Status(Vec::new())
        );
        service.stop();
    }

    /// A holder found at start whose process is no longer in the project is
    /// stopped through the stop policy its record carries, with a notice, and
    /// its row goes once it has exited.
    #[test]
    fn a_removed_process_found_at_start_is_stopped_with_a_notice() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true), ("gone", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
        service.wait_for(&project, "gone", ProcessState::Running);
        service.stop();
        fixture.store.put(&project, commands(&[("api", true)]));

        let successor = fixture.start(Arc::new(fixture.runner()));

        wait_until("the removed process to be stopped", || {
            find(&successor.status(&project), "gone").is_none()
        });
        let notices = successor.notices(&project);
        assert!(
            notices.iter().any(|notice| {
                notice.process().as_ref().map(ProcessName::as_ref) == Some("gone")
                    && notice.message() == "stopped: removed from the project"
            }),
            "{notices:?}"
        );
        assert_eq!(fixture.runner().live_records().len(), 1, "only api runs");
        successor.stop();
    }

    /// A project removed while the service was down has what it still runs
    /// stopped, and is let go once nothing runs.
    #[test]
    fn an_unregistered_project_found_running_is_stopped_and_let_go() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
        service.wait_for(&project, "api", ProcessState::Running);
        service.stop();
        fixture.store.forget(&project);

        let successor = fixture.start(Arc::new(fixture.runner()));

        wait_until("the project to be let go", || {
            successor.send(ServiceRequest::Ps { project: None })
                == ServiceResponse::Status(Vec::new())
        });
        assert!(fixture.runner().live_records().is_empty());
        successor.stop();
    }

    /// New settings reach the next launch only: a process started after the
    /// change gets the new terminal size, and a running one keeps its own.
    #[test]
    fn settings_change_the_size_of_the_next_launch_only() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true), ("web", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
        service.wait_for(&project, "api", ProcessState::Running);
        let bigger = PtySize::builder()
            .rows(Rows::new(40))
            .cols(Cols::new(120))
            .build();

        fixture
            .store
            .apply(&SettingChange::DefaultSize(bigger))
            .unwrap();
        assert_eq!(
            service.send(ServiceRequest::SettingsChanged),
            ServiceResponse::Ok
        );
        service.control(&project, "web", ProcessAction::Start);
        service.wait_for(&project, "web", ProcessState::Running);

        let size_of = |name: &str| {
            let record = fixture
                .runner()
                .live_records()
                .into_iter()
                .find(|record| record.process().as_ref() == name)
                .unwrap();
            greet(
                &mut UnixStream::connect(record.socket()).unwrap(),
                record.socket(),
                ClientRole::Attach { read_only: true },
                WAIT,
            )
            .unwrap()
            .size()
        };
        assert_eq!(size_of("web"), bigger);
        assert_eq!(size_of("api"), *test_settings().default_size());
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
