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
    event_draft::EventDraft,
    event_kind::EventKind,
    event_log::EventLog,
    event_seq::EventSeq,
    event_waiter::EventWaiter,
    flow::Flow,
    holder_pool::HolderPool,
    launch_environment::LaunchEnvironment,
    located::Located,
    notice::Notice,
    notice_level::NoticeLevel,
    notices::Notices,
    port_block::PortBlock,
    port_holder::PortHolder,
    port_owner::PortOwner,
    port_sampler::{PORT_SAMPLE_INTERVAL, PortSampler},
    probe_worker::ProbeWorker,
    process_action::ProcessAction,
    project_ports::ProjectPorts,
    project_status::ProjectStatus,
    protocol,
    record_retry::RecordRetry,
    reply::Reply,
    service_deps::ServiceDeps,
    service_error::ServiceError,
    service_error_body::ServiceErrorBody,
    service_error_kind::ServiceErrorKind,
    service_request::ServiceRequest,
    service_response::ServiceResponse,
    service_version::ServiceVersion,
    shared_event_log::SharedEventLog,
    shared_holders::SharedHolders,
    shared_notices::SharedNotices,
    singleton::Singleton,
    store_recorder::StoreRecorder,
    supervisor::{HolderScreen, ScreenReply, StaleRelease, Supervisor, SupervisorDeps},
    usage_sampler::UsageSampler,
};
use crate::{
    adapter::{holder::HolderRecord, workspace_request::WorkspaceRequest},
    domain::{
        config::{ConfigError, Stack},
        port::{ProcessHandle, RaisedEdits, RunHistory, SettingsReader, StackEditLog, StackReader},
        project::Project,
        pty::PtyError,
        settings::Settings,
        value::{EditId, Pid, ProcessName, ProjectName, ProjectRoot, StackRevision},
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

/// The one service of a user: owns every loaded project's supervisor on a
/// single thread, which every other thread only sends events to. Clients
/// connect on its socket; holders run every process and outlive it. The
/// service only reads the stored stacks and settings; writers tell it what
/// changed.
pub struct ServiceRuntime {
    /// The running binary's version.
    version: ServiceVersion,
    /// The running binary's path.
    binary: PathBuf,
    /// The holders every process runs under.
    holders: Arc<dyn HolderPool>,
    /// Builds each project's driven adapters.
    ports: Box<dyn ProjectPorts>,
    /// Reads each project's stack.
    stacks: Box<dyn StackReader>,
    /// Reads the user's settings.
    settings_store: Box<dyn SettingsReader>,
    /// Reads the agent edits each project's stack took.
    edits: Box<dyn StackEditLog>,
    /// Reads how each process last ended on its own as a project loads.
    runs: Box<dyn RunHistory>,
    /// Reads the newest agent edit an earlier service raised, as a project
    /// loads.
    raised: Box<dyn RaisedEdits>,
    /// Stores run ends and raised edits off the loop; every supervisor shares
    /// it.
    store_recorder: StoreRecorder,
    /// The settings every supervisor launches with, as last read.
    settings: Settings,
    /// Where processes, timers, workers and clients deliver events.
    events: Sender<ServiceEvent>,
    /// Notices of every project and of the service.
    notices: SharedNotices,
    /// The supervisor of every loaded project.
    supervisors: HashMap<ProjectRoot, Supervisor>,
    /// The stored stack revision each loaded supervisor last applied, so a
    /// write whose notice never arrived is still caught.
    stack_revisions: HashMap<ProjectRoot, Option<StackRevision>>,
    /// The newest stored agent edit of each loaded project already raised as
    /// an event, by this service or an earlier one.
    raised_edits: HashMap<ProjectRoot, EditId>,
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
    /// Samples every project's resource usage, when wired.
    usage_sampler: Option<UsageSampler>,
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
        let mut acceptor = Acceptor::bind(&deps.socket, sender.clone())?;
        let mut runtime = Self::new(deps, settings, sender, &state_dir);
        runtime.adopt_running();
        let exit = runtime.serve(&events);
        runtime.shutdown();
        acceptor.close();
        runtime.detach_launches(&events);
        runtime.supervisors.clear();
        runtime.store_recorder.flush();
        runtime
            .notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush();
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
            socket: _,
            holders,
            ports,
            metrics,
            stacks,
            settings: settings_store,
            edits,
            runs,
            raised,
            run_writes,
            raised_writes,
            listening,
        } = deps;
        let notices = Notices::shared(Some(protocol::log_path(state_dir)));
        Self {
            version,
            binary,
            holders,
            ports,
            stacks,
            settings_store,
            edits,
            store_recorder: StoreRecorder::spawn(
                run_writes,
                raised_writes,
                Arc::clone(&notices),
                RecordRetry::service(),
            ),
            runs,
            raised,
            settings,
            probes: ProbeWorker::spawn(events.clone()),
            port_sampler: listening.map(|listening| PortSampler::spawn(listening, events.clone())),
            ports_deadline: Instant::now(),
            usage_sampler: metrics.map(UsageSampler::new),
            events,
            notices,
            supervisors: HashMap::new(),
            stack_revisions: HashMap::new(),
            raised_edits: HashMap::new(),
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

    /// The earliest deadline of any supervisor's waits or readiness checks, of
    /// any client waiting for events, and of the next usage and port sample.
    fn next_deadline(&self) -> Option<Instant> {
        let supervisors = self
            .supervisors
            .values()
            .map(Supervisor::next_wait_deadline);
        let usage = self
            .usage_sampler
            .as_ref()
            .and_then(|sampler| sampler.next_deadline(self.supervisors.values()));
        let events = self
            .event_waiters
            .iter()
            .map(|waiter| Some(waiter.deadline));
        let ports = self
            .port_sampler
            .as_ref()
            .filter(|_| !self.live_root_pids().is_empty())
            .map(|_| self.ports_deadline);
        supervisors
            .chain(events)
            .chain([usage, ports])
            .flatten()
            .min()
    }

    /// Acknowledges the held exits whose restart settled, samples usage and
    /// ports where due, settles waits, answers clients waiting for events,
    /// and lets go of an unregistered project once it runs nothing.
    fn tick(&mut self, now: Instant) {
        for supervisor in self.supervisors.values_mut() {
            supervisor.release_settled_exits();
            supervisor.settle_waits(now);
        }
        if let Some(sampler) = &mut self.usage_sampler {
            sampler.sample(now, self.supervisors.values_mut());
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
            self.stack_revisions.remove(&root);
            self.raised_edits.remove(&root);
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
            ServiceEvent::LaunchBlocked {
                project,
                pane,
                generation,
                port,
                occupant,
            } => {
                let lineage = occupant.lineage();
                let holder = self
                    .supervisors
                    .values()
                    .find_map(|supervisor| supervisor.holder_of_pids(&lineage))
                    .map_or(PortOwner::Other(occupant), PortOwner::Supervised);
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    let block = PortBlock::builder().port(port).holder(holder).build();
                    supervisor.handle_launch_blocked(pane, generation, block);
                }
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
            ServiceEvent::AdoptionRetryDue {
                project,
                pane,
                attempt,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_adoption_retry_due(pane, attempt);
                }
            },
            ServiceEvent::ReconnectLocated {
                project,
                pane,
                generation,
                record,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_reconnect_located(
                        pane,
                        generation,
                        record.map(|record| *record),
                    );
                }
            },
            ServiceEvent::LogTailRead {
                project,
                pane,
                read,
                tail,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_log_tail(pane, read, tail);
                }
            },
            ServiceEvent::AdoptionProbed {
                project,
                pane,
                attempt,
                asked,
                record,
            } => {
                if let Some(supervisor) = self.supervisors.get_mut(&project) {
                    supervisor.handle_adoption_probed(
                        pane,
                        attempt,
                        asked,
                        record.map(|record| *record),
                    );
                }
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
                self.share_held_ports();
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
        match &request {
            ServiceRequest::Up { project, .. } | ServiceRequest::Control { project, .. } => {
                self.refresh_stack(project)?;
                self.supervisor(project)?;
                self.share_held_ports();
            },
            ServiceRequest::Ps {
                project: Some(project),
            }
            | ServiceRequest::Events { project, .. } => self.refresh_stack(project)?,
            ServiceRequest::Ps { project: None } => {
                let loaded: Vec<ProjectRoot> = self.supervisors.keys().cloned().collect();
                for project in &loaded {
                    self.refresh_stack(project)?;
                }
            },
            ServiceRequest::Hello { .. }
            | ServiceRequest::Down { .. }
            | ServiceRequest::Wait { .. }
            | ServiceRequest::Locate { .. }
            | ServiceRequest::Log { .. }
            | ServiceRequest::Screen { .. }
            | ServiceRequest::Notices { .. }
            | ServiceRequest::Workspace { .. }
            | ServiceRequest::StackChanged { .. }
            | ServiceRequest::SettingsChanged
            | ServiceRequest::Replace
            | ServiceRequest::Stop => {},
        }
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
                let mut prepared = supervisor.prepare_wait(&name, &wait)?;
                let fresh = match action {
                    ProcessAction::Start if supervisor.runs_or_starts(&name) => {
                        prepared = prepared.on_a_running_process();
                        false
                    },
                    ProcessAction::Start | ProcessAction::Restart => true,
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
        let revision = self.stacks.stack_revision(project).map_err(store_refusal)?;
        let stored = self.stacks.stack(project).map_err(store_refusal)?;
        self.stack_revisions.insert(project.clone(), revision);
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
        self.raise_edits(project)
    }

    /// Raises each agent edit stored for `project` since the newest one
    /// already raised as a `notice` event, by this service or an earlier one
    /// whose mark is stored, at the time it was recorded or the newest
    /// event's when that is later. An edit an earlier service raised is never
    /// raised again, so no agent wakes to one it could already have seen; one
    /// stored while no service ran is. A project no longer registered has
    /// none, and starts afresh if it is registered again.
    ///
    /// # Errors
    /// Returns the refusal when the edit log or the stored mark cannot be
    /// read.
    fn raise_edits(&mut self, project: &ProjectRoot) -> Result<(), ServiceErrorBody> {
        let edits = self.edits.edits(project).map_err(store_refusal)?;
        let raised = match self.raised_edits.get(project) {
            Some(raised) => Some(*raised),
            None => self.raised.raised_through(project).map_err(store_refusal)?,
        };
        let mut log = self
            .event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for edit in edits
            .iter()
            .filter(|edit| raised.is_none_or(|raised| *edit.id() > raised))
        {
            log.record_at(
                EventDraft::builder()
                    .project(Some(project.clone()))
                    .process(edit.process().clone())
                    .kind(EventKind::Notice)
                    .detail(edit.message().clone())
                    .build(),
                *edit.at(),
            );
        }
        drop(log);
        match edits.last() {
            Some(newest) => {
                if raised != Some(*newest.id()) {
                    self.store_recorder.edits_raised(project, *newest.id());
                }
                self.raised_edits.insert(project.clone(), *newest.id())
            },
            None => self.raised_edits.remove(project),
        };
        Ok(())
    }

    /// Applies `project`'s stored stack again when it is loaded and the store
    /// moved past the revision its supervisor last applied, which a writer
    /// that committed but never told the service leaves behind.
    ///
    /// # Errors
    /// Returns the refusal when the revision or stack cannot be read.
    fn refresh_stack(&mut self, project: &ProjectRoot) -> Result<(), ServiceErrorBody> {
        let Some(applied) = self.stack_revisions.get(project).copied() else {
            return Ok(());
        };
        if self.stacks.stack_revision(project).map_err(store_refusal)? == applied {
            return Ok(());
        }
        self.stack_changed(project)
    }

    /// Reads the stored settings for every later launch, hands them to every
    /// supervisor, and raises the agent's setting change each loaded
    /// project's edit log took with it.
    ///
    /// # Errors
    /// Returns the refusal when the settings or an edit log cannot be read.
    fn settings_changed(&mut self) -> Result<(), ServiceErrorBody> {
        self.settings = self.settings_store.load().map_err(store_refusal)?;
        for supervisor in self.supervisors.values_mut() {
            supervisor.apply_settings(&self.settings);
        }
        let loaded: Vec<ProjectRoot> = self.supervisors.keys().cloned().collect();
        for project in &loaded {
            self.raise_edits(project)?;
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

    /// Tells every supervisor which ports the supervised processes of every
    /// project listen on, so a start can refuse a port another one holds.
    fn share_held_ports(&mut self) {
        let held: HashMap<u16, PortHolder> = self
            .supervisors
            .values()
            .flat_map(Supervisor::held_ports)
            .collect();
        for supervisor in self.supervisors.values_mut() {
            supervisor.set_held_ports(held.clone());
        }
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
        let revision = self.stacks.stack_revision(project).map_err(store_refusal)?;
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
        let ended = self.runs.ended_runs(project).map_err(store_refusal)?;
        self.raise_edits(project)?;
        let shared = SharedHolders::new(Arc::clone(&self.holders));
        let deps = SupervisorDeps::builder()
            .runner(Box::new(shared.clone()))
            .events(self.events.clone())
            .notices(Arc::clone(&self.notices))
            .event_log(Arc::clone(&self.event_log))
            .probes(Some(self.probes.clone()))
            .notifier(self.ports.notifier())
            .port_occupants(self.ports.port_occupants())
            .settings(self.settings.clone())
            .runs(Some(self.store_recorder.clone()))
            .build();
        let mut supervisor =
            Supervisor::new(stack.project().clone(), &stack, deps).with_adopter(Box::new(shared));
        supervisor.restore_ends(&ended);
        for record in held {
            supervisor.adopt(record.clone());
        }
        self.supervisors.insert(project.clone(), supervisor);
        self.stack_revisions.insert(project.clone(), revision);
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
                Ok(
                    event @ (ServiceEvent::Spawned { .. }
                    | ServiceEvent::LaunchBlocked { .. }
                    | ServiceEvent::Adopted { .. }),
                ) => {
                    self.handle(event);
                },
                Ok(_) => {},
                Err(_) => return,
            }
        }
    }

    /// Answers a read of `project`'s events after `since` of `kinds`: at once
    /// when there are any, `wait` is zero or the page has a gap, as a cursor
    /// of an earlier service's log has, otherwise when the next one arrives
    /// or `wait` passes.
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
        if !page.events().is_empty() || wait.is_zero() || *page.gap() {
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
    fn live_root_pids(&self) -> Vec<Pid> {
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
                    .process(None)
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
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::{
        adapter::{
            holder::{ClientMessage, ClientRole, HolderId, LogQuery, client::greet, read_log},
            ipc,
            service::{
                BatchOutcome, EventPage, StateGoal, WaitCondition, WaitSpec, WaitStatus,
                fake_pool::FakePool,
                null_sink::NullSink,
                running::Running,
                runtime_fixture::Fixture,
                runtime_testing::{
                    POLL, SLEEPER, TEST_BINARY, VERSION, WAIT, commands, find, no_env, process,
                    sleeper, wait_until,
                },
                supervisor::{
                    ChannelOutputSink, SpawnGeneration,
                    testing::{KillCountHandle, Orphan, test_settings, test_stop},
                },
            },
            signal_target::SignalTarget,
            test_wait::REPLY,
        },
        domain::{
            config::SettingChange,
            coordination::Author,
            port::{ProcessRunner, SettingsStore},
            process::{
                LogPattern, PatternKind, PatternText, ProcessState, Readiness, ReadyCheck,
                ReadyProbe,
            },
            pty::{ProcessOutput, PtySize},
            value::{CommandLine, PaneId, PositiveDuration},
        },
    };

    /// Longest the loop may take to answer a request while a holder read,
    /// launch or adoption is stalled, far below the holder timeouts.
    const PROMPT_ANSWER: Duration = Duration::from_secs(2);
    /// How long a gated holder waits after the service is told to stop,
    /// well inside the holder ready timeout.
    const GATE_DELAY: Duration = Duration::from_millis(300);
    /// A child that waits for its supervisor to connect, then writes a fixed
    /// flood with a marker in its middle, far past what the event queue and
    /// a holder's outbox hold, then waits.
    const FLOOD: &str = "sleep 1; head -c 3000000 /dev/zero | tr '\\0' x; echo; echo MARKER-READY; \
                         head -c 3000000 /dev/zero | tr '\\0' y; sleep 600";
    /// What [`FLOOD`] writes in all, each newline made two bytes by the
    /// terminal.
    const FLOOD_BYTES: usize = 3_000_000 + 2 + 14 + 3_000_000;
    /// The marker in the middle of [`FLOOD`].
    const FLOOD_MARKER: &[u8] = b"MARKER-READY";
    /// Longest a flood may take to reach its log, on a loaded machine.
    const FLOOD_WAIT: Duration = Duration::from_secs(60);
    /// How a resync screen starts: a full terminal reset.
    const RESYNC_PREFIX: &[u8] = b"\x1bc";

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
        let socket = service.socket().clone();

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
        let stalled = fixture.scratch().path().join("stalled.sock");
        let _never_accepts = UnixListener::bind(&stalled).unwrap();
        let pool = FakePool::new(Vec::new(), Some(stalled));
        let service = fixture.start(Arc::new(pool));
        service.up(&project);

        let socket = service.socket().clone();
        service.wait_for(&project, "api", ProcessState::Running);
        let screen_project = project.clone();
        let reading = thread::spawn(move || {
            protocol::send_within(
                &socket,
                &ServiceRequest::Screen {
                    project: screen_project,
                    name: ProcessName::try_new("api").unwrap(),
                },
                REPLY,
            )
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
        *pool.legacy_result().lock().unwrap() = Some(io::Error::other("read-only file system"));
        let removals = Arc::clone(pool.legacy_removals());

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
        child_pid: Pid,
    ) -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(project.clone())
            .process(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(SLEEPER).unwrap())
            .stop(test_stop())
            .socket(fixture.scratch().path().join("gone.sock"))
            .holder_pid(Pid::try_new(process::id()).unwrap())
            .child_pid(child_pid)
            .child_start_token(None)
            .log(fixture.scratch().path().join("api.log"))
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
        let spawns = Arc::clone(pool.spawns());
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
            let kills = Arc::clone(pool.kills());
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
        let spawns = Arc::clone(pool.spawns());
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
        assert!(SignalTarget::Process(pid).is_alive(), "the child lives on");
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
        let spawns = Arc::clone(pool.spawns());
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
    /// the holder resyncs it with a fresh screen. The output that resync
    /// dropped is read back from the log, so once the loop reads again it
    /// sees every byte in order with no gap, the marker in the middle too.
    #[test]
    fn a_flood_waits_on_the_bounded_queue_and_its_resync_drops_no_byte() {
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
        let flood_bytes = u64::try_from(FLOOD_BYTES).unwrap();
        let started = Instant::now();
        while handle
            .output_end()
            .is_none_or(|end| end.offset() < flood_bytes)
        {
            assert!(started.elapsed() < FLOOD_WAIT, "the flood never finished");
            thread::sleep(POLL);
        }
        assert_eq!(
            events.len(),
            EVENT_CAPACITY,
            "the queue is full and no fuller"
        );

        let mut seen = Vec::new();
        let mut resynced = false;
        while seen.len() < FLOOD_BYTES {
            let Ok(ServiceEvent::Output { output, .. }) = events.recv_timeout(WAIT) else {
                panic!("the flood stopped at {} bytes", seen.len());
            };
            match output {
                ProcessOutput::Placed { at, bytes } => {
                    let at = usize::try_from(at.offset()).unwrap();
                    assert!(at <= seen.len(), "a gap from {} to {at}", seen.len());
                    seen.extend_from_slice(bytes.get(seen.len() - at..).unwrap_or_default());
                },
                ProcessOutput::Repaint { bytes, .. } if bytes.starts_with(RESYNC_PREFIX) => {
                    resynced = true;
                },
                _ => {},
            }
        }
        assert!(
            resynced,
            "the holder resynced the connection it could not feed"
        );
        assert!(
            seen.windows(FLOOD_MARKER.len())
                .any(|window| window == FLOOD_MARKER)
        );
        handle.kill().unwrap();
    }

    /// A project named by a relative path is refused as unreadable, since it
    /// would resolve against the service's directory.
    #[test]
    fn a_relative_project_is_refused() {
        let fixture = Fixture::new();
        let service = fixture.start(Arc::new(fixture.runner()));

        let response: ServiceResponse = ipc::request_within(
            service.socket(),
            &serde_json::json!({ "ps": { "project": "project" } }),
            REPLY,
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
                version: "9.9.9".parse().unwrap()
            }),
            ServiceResponse::Hello {
                version: VERSION.parse().unwrap(),
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
        assert!(!started.already_running());

        let running = WaitSpec::builder()
            .until(Some(WaitCondition::State(StateGoal::Running)))
            .timeout(WAIT)
            .build();
        let ServiceResponse::Waited(again) = service.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new("api").unwrap(),
            action: ProcessAction::Start,
            environment: no_env(),
            wait: Some(running),
        }) else {
            panic!("a start with a wait answers how the wait ended");
        };
        assert_eq!(again.status(), WaitStatus::Matched);
        assert!(again.already_running(), "{again:?}");

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

    /// A readiness marker printed while no service ran, inside the readiness
    /// window, makes the adopted run ready, and a log wait from a cursor taken
    /// before the restart still matches it, at a cursor that never went back.
    #[test]
    fn output_printed_while_the_service_was_down_reaches_readiness_and_waits() {
        let fixture = Fixture::new();
        let folder = fixture.folder("shop");
        let go = folder.as_ref().join("go");
        let marked = sleeper("api", false)
            .with_command(
                CommandLine::try_new(format!(
                    "echo booting; while [ ! -e '{}' ]; do sleep 0.05; done; echo READY now; \
                     sleep 600",
                    go.display()
                ))
                .unwrap(),
            )
            .with_ready(Some(
                ReadyCheck::builder()
                    .probe(ReadyProbe::from_stored("log", "READY").unwrap())
                    .timeout(PositiveDuration::try_new(WAIT * 4).unwrap())
                    .build(),
            ));
        let project = fixture.project_in("shop", vec![marked]);
        let api = ProcessName::try_new("api").unwrap();
        let service = fixture.start(Arc::new(fixture.runner()));
        let control = ServiceRequest::Control {
            project: project.clone(),
            name: api.clone(),
            action: ProcessAction::Start,
            environment: no_env(),
            wait: Some(
                WaitSpec::builder()
                    .until(Some(WaitCondition::Log(
                        LogPattern::builder()
                            .text(PatternText::try_new("booting").unwrap())
                            .kind(PatternKind::Substring)
                            .build(),
                    )))
                    .timeout(WAIT)
                    .build(),
            ),
        };
        let ServiceResponse::Waited(booted) = service.send(control) else {
            panic!("a start with a wait answers how the wait ended");
        };
        assert_eq!(booted.status(), WaitStatus::Matched);
        let before = *booted.cursor();
        assert_eq!(
            *process(&service.status(&project), "api").ready(),
            Readiness::Waiting
        );
        service.stop();

        fs::write(&go, "").unwrap();
        let log = fixture.dirs().process_log(&project, &api);
        wait_until("the marker in the log", || {
            fs::read_to_string(&log).is_ok_and(|text| text.contains("READY now"))
        });
        let adopting = fixture.start(Arc::new(fixture.runner()));
        wait_until("the adopted run to be ready", || {
            *process(&adopting.status(&project), "api").ready() == Readiness::Ready
        });
        let ServiceResponse::Waited(marker) = adopting.send(ServiceRequest::Wait {
            project: project.clone(),
            name: api.clone(),
            wait: WaitSpec::builder()
                .until(Some(WaitCondition::Log(
                    LogPattern::builder()
                        .text(PatternText::try_new("READY now").unwrap())
                        .kind(PatternKind::Substring)
                        .build(),
                )))
                .since(Some(before))
                .timeout(WAIT)
                .build(),
        }) else {
            panic!("a wait answers how it ended");
        };
        assert_eq!(marker.status(), WaitStatus::Matched, "{marker:?}");
        assert!(*marker.matched().as_ref().unwrap().at() >= before);
        assert!(*marker.cursor() > before, "the cursor never goes back");
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
        let file_name = log.file_name().unwrap().to_string_lossy();
        assert!(
            file_name.starts_with("worker.") && file_name.ends_with(".log"),
            "{file_name}"
        );

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
            .store()
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

    /// The `notice` events of `project` after `since`.
    fn edit_events(service: &Running, project: &ProjectRoot, since: Option<EventSeq>) -> EventPage {
        let ServiceResponse::Events(page) = service.send(ServiceRequest::Events {
            project: project.clone(),
            since,
            kinds: vec![EventKind::Notice],
            wait: Duration::ZERO,
        }) else {
            panic!("events answer with a page");
        };
        page
    }

    /// The detail and process of each event of `page`.
    fn told(page: &EventPage) -> Vec<(String, Option<String>)> {
        page.events()
            .iter()
            .map(|event| {
                (
                    event.detail().clone(),
                    event.process().as_ref().map(ToString::to_string),
                )
            })
            .collect()
    }

    /// Every stored agent edit no service raised is a `notice` event on its
    /// process, in cursor order with the other events and times that never
    /// go back: one stored before the project loads arrives as it loads, a
    /// later one with the writer's stack change, and one whose change notice
    /// was missed with the next read; none is raised twice. A service started
    /// later raises again none of them, only the one stored while no service
    /// ran.
    #[test]
    fn a_stored_agent_edit_is_a_notice_event_in_cursor_order() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", false)]));
        fixture
            .store()
            .record_edit(&project, "api", "agent claude added 'api'");
        let service = fixture.start(Arc::new(fixture.runner()));

        let loaded = edit_events(&service, &project, None);
        assert_eq!(told(&loaded), [(
            "agent claude added 'api'".to_string(),
            Some("api".to_string())
        )]);

        fixture
            .store()
            .record_edit(&project, "api", "agent claude changed autostart of 'api'");
        for _ in 0..2 {
            assert_eq!(
                service.send(ServiceRequest::StackChanged {
                    project: project.clone()
                }),
                ServiceResponse::Ok
            );
        }
        let changed = edit_events(&service, &project, Some(*loaded.cursor()));
        assert_eq!(told(&changed), [(
            "agent claude changed autostart of 'api'".to_string(),
            Some("api".to_string())
        )]);
        assert!(changed.events()[0].seq().is_after(loaded.events()[0].seq()));
        assert!(changed.events()[0].at() >= loaded.events()[0].at());

        fixture
            .store()
            .record_edit(&project, "api", "agent claude removed 'api'");
        let missed = edit_events(&service, &project, Some(*changed.cursor()));
        assert_eq!(told(&missed), [(
            "agent claude removed 'api'".to_string(),
            Some("api".to_string())
        )]);
        assert!(
            edit_events(&service, &project, Some(*missed.cursor()))
                .events()
                .is_empty()
        );
        service.stop();

        fixture
            .store()
            .record_edit(&project, "api", "agent claude added 'web'");
        let successor = fixture.start(Arc::new(fixture.runner()));
        assert_eq!(
            told(&edit_events(&successor, &project, None)),
            [(
                "agent claude added 'web'".to_string(),
                Some("api".to_string())
            )],
            "only the edit no service raised"
        );
        successor.stop();
    }

    /// An agent's setting change, which moves no stack's revision, is a
    /// `notice` event on no process once the service is told of it.
    #[test]
    fn an_agents_setting_change_is_a_notice_event() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", false)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        let loaded = edit_events(&service, &project, None);

        fixture
            .store()
            .apply(
                &SettingChange::DefaultSize(PtySize::try_new(40, 120).unwrap()),
                &Author::agent("claude"),
            )
            .unwrap();
        assert_eq!(
            service.send(ServiceRequest::SettingsChanged),
            ServiceResponse::Ok
        );

        let told = told(&edit_events(&service, &project, Some(*loaded.cursor())));
        assert_eq!(told, [(
            "agent claude set default-size to 40x120".to_string(),
            None
        )]);
        service.stop();
    }

    /// A cursor of an earlier service's log, read plain or with a wait, gets
    /// at once a gap and every event the new service holds, never a long
    /// poll; so does a cursor past the newest event.
    #[test]
    fn a_cursor_from_an_earlier_service_gets_a_gap_at_once() {
        let fixture = Fixture::new();
        let project = fixture.project(commands(&[("api", true)]));
        let service = fixture.start(Arc::new(fixture.runner()));
        service.up(&project);
        service.wait_for(&project, "api", ProcessState::Running);
        let read = |service: &Running, since: Option<EventSeq>, wait: Duration| {
            let ServiceResponse::Events(page) = service.send(ServiceRequest::Events {
                project: project.clone(),
                since,
                kinds: Vec::new(),
                wait,
            }) else {
                panic!("events answer with a page");
            };
            page
        };
        let before = *read(&service, None, Duration::ZERO).cursor();
        service.stop();

        let successor = fixture.start(Arc::new(fixture.runner()));
        successor.wait_for(&project, "api", ProcessState::Running);
        let held = read(&successor, None, Duration::ZERO);
        let future = EventSeq::builder()
            .epoch(held.cursor().epoch())
            .seq(held.cursor().seq() + 5)
            .build();
        for (since, wait) in [(before, Duration::ZERO), (before, WAIT), (future, WAIT)] {
            let asked = Instant::now();
            let page = read(&successor, Some(since), wait);
            assert!(asked.elapsed() < PROMPT_ANSWER, "{since} waited");
            assert!(page.gap(), "{since}");
            assert_eq!(page.events(), held.events(), "{since}");
            assert_ne!(page.cursor().epoch(), before.epoch());
        }
        successor.stop();
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
        fixture.store().put(&project, commands(&[("api", true)]));

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
        fixture.store().forget(&project);

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
        let bigger = PtySize::try_new(40, 120).unwrap();

        fixture
            .store()
            .apply(&SettingChange::DefaultSize(bigger), &Author::human())
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

        release_unrecorded(Ok(Box::new(
            KillCountHandle::builder().kills(Arc::clone(&kills)).build(),
        )));

        assert_eq!(kills.load(Ordering::SeqCst), 1);
    }
}
