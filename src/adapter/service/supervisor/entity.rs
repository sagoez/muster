use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::PoisonError,
    time::{Instant, SystemTime},
};

use crossbeam_channel::Sender;
use getset::{Getters, MutGetters};

use super::{
    adopter::HolderAdopter, config_membership::ConfigMembership, deps::SupervisorDeps,
    metrics::METRICS_SAMPLE_INTERVAL, pane::Pane, pending_launch::PendingLaunch,
    spawn_generation::SpawnGeneration,
};
use crate::{
    adapter::service::{
        Notice, NoticeLevel, ProcessStatus, ProcessUsage, ProjectKey, ProjectStatus, Refusal,
        ServiceEvent, SharedNotices,
    },
    application::Workspace,
    domain::{
        agent_session::AgentSessionId,
        color_scheme::ColorScheme,
        config::ConfigError,
        notification::NotificationScope,
        port::{
            AgentSessionStore, ConfigWatcher, Notifier, ProcessHandle, ProcessMetrics,
            ProcessRunner, ProjectRegistry,
        },
        process::ProcessState,
        pty::{ExitOutcome, PtySize},
        value::{PaneId, ProcessName},
    },
};

/// Shown when a process name matches nothing in the project.
pub(super) const UNKNOWN_PROCESS_NOTICE: &str = "no process with that name in this project";

/// Supervises one project's processes: spawns them, applies their stop and
/// restart policies, adopts holders a previous service left running, and
/// reports failures as notices rather than panicking.
#[derive(Getters, MutGetters)]
pub struct Supervisor {
    /// The project this supervisor runs.
    #[getset(get = "pub")]
    pub(super) project: ProjectKey,
    /// The project's processes and their lifecycle state.
    #[getset(get = "pub")]
    pub(super) workspace: Workspace,
    /// Spawns each process.
    pub(super) runner: Box<dyn ProcessRunner>,
    /// Takes over holders a previous service left running.
    pub(super) adopter: Option<Box<dyn HolderAdopter>>,
    /// Reads and locks the project's config.
    #[getset(get = "pub(crate)")]
    pub(super) registry: Box<dyn ProjectRegistry>,
    /// Durable agent-session history.
    pub(super) agent_session_store: Option<Box<dyn AgentSessionStore>>,
    /// Where output, restart timers and stop deadlines are delivered.
    pub(super) events: Sender<ServiceEvent>,
    /// Where supervision reports what the user should know.
    pub(super) notices: SharedNotices,
    /// Reports edits to the project's config, so it is reconciled live.
    pub(super) config_watcher: Option<Box<dyn ConfigWatcher>>,
    /// Session reopens waiting for the session's previous pane to finish
    /// retiring, keyed by that pane.
    pub(super) pending_session_reopens: HashMap<PaneId, AgentSessionId>,
    /// The terminal and live handle of every pane that has run.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    pub(super) panes: HashMap<PaneId, Pane>,
    /// The current spawn generation of every pane, so stale events are dropped.
    #[getset(get = "pub(crate)")]
    pub(super) generations: HashMap<PaneId, SpawnGeneration>,
    /// Consecutive quick automatic restarts, which set the backoff delay.
    pub(super) restart_attempts: HashMap<PaneId, u32>,
    /// Restarts of each pane, by its policy or on request.
    pub(super) restarts: HashMap<PaneId, u32>,
    /// How each pane's most recent child ended, shown by `ps` while it is
    /// not running.
    pub(super) last_exits: HashMap<PaneId, ExitOutcome>,
    /// Panes in a crash streak, whose first crash was told and whose later
    /// ones are not, until the streak ends.
    pub(super) crash_streaks: HashSet<PaneId>,
    /// While a project-wide request runs, each error raised about one of its
    /// processes, which is why that process was left alone.
    pub(super) batch_refusals: RefCell<Option<Vec<Refusal>>>,
    /// Panes whose last spawn failed, so a failure streak is reported once.
    pub(super) failing_spawns: HashSet<PaneId>,
    /// Launches and adoptions handed to a worker and not yet settled, keyed by
    /// the starting pane.
    pub(super) launches: HashMap<PaneId, PendingLaunch>,
    /// Set by [`Self::shutdown`]: from then on the result of a launch the
    /// shutdown superseded is detached, never stopped, since the service is
    /// letting go of every process.
    pub(super) detaching: bool,
    /// Latest sampled resource usage of every running pane.
    #[getset(get = "pub(crate)")]
    pub(super) usage: HashMap<PaneId, ProcessUsage>,
    /// Samples each running process's resource usage.
    pub(super) process_metrics: Option<Box<dyn ProcessMetrics>>,
    /// When usage is next resampled.
    pub(super) metrics_deadline: Instant,
    /// Delivers desktop notifications.
    pub(super) notifier: Option<Box<dyn Notifier>>,
    /// Whether attention a process raises also reaches the desktop.
    pub(super) desktop_notifications: bool,
    /// Panes whose last holder screen read failed, so a failure streak is
    /// reported once.
    pub(super) failing_screen_reads: HashSet<PaneId>,
    /// Writable clients attached to each pane's terminal.
    pub(super) attached: HashMap<PaneId, u32>,
    /// The handle of each pane's exited process whose exit is not yet
    /// acknowledged, held while its restart is pending so a service that
    /// stops meanwhile leaves the exit for its successor to apply again.
    pub(super) held_exits: HashMap<PaneId, Box<dyn ProcessHandle>>,
    /// Terminal size each new process starts with.
    pub(super) size: PtySize,
    /// The default colors each process's terminal reports when asked.
    pub(super) color_scheme: ColorScheme,
    /// Monotonic source for terminal-lifetime notification scopes.
    pub(super) next_notification_scope: NotificationScope,
}

impl Supervisor {
    /// Loads `project`'s config and creates its supervisor. Nothing starts
    /// until [`Self::up`] or a start request.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project's config cannot be read.
    pub fn load(project: ProjectKey, deps: SupervisorDeps) -> Result<Self, ConfigError> {
        let config = deps.registry.workspace(project.as_ref())?;
        let workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        Ok(Self::new(project, workspace, deps))
    }

    /// Creates a supervisor for `project` over an already loaded `workspace`.
    pub fn new(project: ProjectKey, workspace: Workspace, deps: SupervisorDeps) -> Self {
        let SupervisorDeps {
            runner,
            registry,
            agent_session_store,
            events,
            notices,
            config_watcher,
            notifier,
            desktop_notifications,
            process_metrics,
            size,
            color_scheme,
        } = deps;
        let mut supervisor = Self {
            project,
            workspace,
            runner,
            adopter: None,
            registry,
            agent_session_store,
            events,
            notices,
            config_watcher,
            pending_session_reopens: HashMap::new(),
            panes: HashMap::new(),
            generations: HashMap::new(),
            restart_attempts: HashMap::new(),
            restarts: HashMap::new(),
            last_exits: HashMap::new(),
            crash_streaks: HashSet::new(),
            batch_refusals: RefCell::new(None),
            failing_spawns: HashSet::new(),
            launches: HashMap::new(),
            detaching: false,
            usage: HashMap::new(),
            process_metrics,
            metrics_deadline: Instant::now() + METRICS_SAMPLE_INTERVAL,
            notifier,
            desktop_notifications,
            failing_screen_reads: HashSet::new(),
            attached: HashMap::new(),
            held_exits: HashMap::new(),
            size,
            color_scheme,
            next_notification_scope: NotificationScope::new(0),
        };
        supervisor.rewatch_config();
        supervisor
    }

    /// Lets this supervisor take over holders a previous service left running.
    pub fn with_adopter(mut self, adopter: Box<dyn HolderAdopter>) -> Self {
        self.adopter = Some(adopter);
        self
    }

    /// Swaps in durable agent-session history, for a test to inject a store.
    #[cfg(test)]
    pub(crate) fn set_agent_session_store(&mut self, store: Box<dyn AgentSessionStore>) {
        self.agent_session_store = Some(store);
    }

    /// Swaps in desktop notification delivery, for a test to inject a fake.
    #[cfg(test)]
    pub(crate) fn set_notifier(&mut self, notifier: Box<dyn Notifier>) {
        self.notifier = Some(notifier);
    }

    /// Swaps in a resource-usage sampler, for a test to inject a fake.
    #[cfg(test)]
    pub(crate) fn set_process_metrics(&mut self, metrics: Box<dyn ProcessMetrics>) {
        self.process_metrics = Some(metrics);
    }

    /// The project's processes, for a test to arrange state directly.
    #[cfg(test)]
    pub(crate) fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    /// Resizes every pane's terminal and live process to `size`, which new
    /// processes then start with.
    pub fn resize(&mut self, size: PtySize) {
        self.size = size;
        for pane in self.panes.values_mut() {
            pane.parser.set_size(size.rows(), size.cols());
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.resize(size);
            }
        }
    }

    /// A point-in-time view of every process, for `ps`. An unsupervised child
    /// found gone is shown as it will settle: exited, or no row at all when its
    /// row was removed from the config, so `ps` never shows an unsupervised
    /// process with no pid.
    pub fn snapshot(&self) -> ProjectStatus {
        let processes = self
            .workspace
            .processes()
            .iter()
            .filter_map(|process| {
                let pane = *process.id();
                let mut state = *process.state();
                if state == ProcessState::Unsupervised
                    && self.running_unsupervised_pid(pane).is_none()
                {
                    let retiring = self.panes.get(&pane).is_some_and(|target| {
                        target.config_membership == ConfigMembership::RetireOnExit
                    });
                    if retiring {
                        return None;
                    }
                    state = ProcessState::Exited;
                }
                let live = self
                    .panes
                    .get(&pane)
                    .filter(|target| target.handle.is_some());
                Some(
                    ProcessStatus::builder()
                        .name(process.name().clone())
                        .kind(*process.kind())
                        .state(state)
                        .activity(*process.activity())
                        .pid(
                            live.and_then(|target| {
                                target
                                    .handle
                                    .as_ref()
                                    .and_then(|handle| handle.process_id())
                            })
                            .or_else(|| self.running_unsupervised_pid(pane)),
                        )
                        .uptime(live.map(|target| target.started_at.elapsed()))
                        .restarts(self.restarts.get(&pane).copied().unwrap_or(0))
                        .usage(self.usage.get(&pane).copied())
                        .attached(self.attached.get(&pane).copied().unwrap_or(0))
                        .last_exit(
                            (!state.is_active() || state == ProcessState::Restarting)
                                .then(|| self.last_exits.get(&pane).cloned())
                                .flatten(),
                        )
                        .stop_grace(
                            process
                                .effective_stop_policy()
                                .map(|policy| *policy.grace_period()),
                        )
                        .build(),
                )
            })
            .collect();
        ProjectStatus::builder()
            .project(self.project.clone())
            .processes(processes)
            .build()
    }

    /// Records how many writable clients are attached to `name`'s terminal.
    #[cfg(test)]
    pub fn set_attached(&mut self, name: &ProcessName, writable_clients: u32) {
        if let Some(pane) = self.pane_named(name) {
            self.set_pane_attached(pane, writable_clients);
        }
    }

    /// Records how many writable clients are attached to `pane`'s terminal,
    /// as its holder reports them.
    pub(super) fn set_pane_attached(&mut self, pane: PaneId, writable_clients: u32) {
        if writable_clients == 0 {
            self.attached.remove(&pane);
        } else {
            self.attached.insert(pane, writable_clients);
        }
    }

    /// Lets go of every process without stopping it: each handle is dropped,
    /// so a holder keeps its process running for the next service to adopt.
    /// Every generation advances, including that of a process with a restart
    /// pending but no terminal, so a timer still in flight is stale. A launch
    /// or adoption in flight is not cancelled: its result, once it comes back
    /// (see [`Self::launches_in_flight`]), is detached the same way. An exit
    /// held for a pending restart is left unacknowledged, so the next service
    /// applies it again, unless its respawn is in flight and settles it.
    pub fn shutdown(&mut self) {
        self.detaching = true;
        for pane in self.panes.values_mut() {
            pane.handle = None;
        }
        let respawning: HashSet<PaneId> = self
            .held_exits
            .keys()
            .copied()
            .filter(|pane| self.is_spawning(*pane))
            .collect();
        self.held_exits.retain(|pane, _| respawning.contains(pane));
        let known: HashSet<PaneId> = self
            .generations
            .keys()
            .chain(self.panes.keys())
            .copied()
            .collect();
        for pane in known {
            self.advance_generation(pane);
        }
    }

    /// Whether the project has a process named `name`.
    pub fn has_process(&self, name: &ProcessName) -> bool {
        self.pane_named(name).is_some()
    }

    /// The name of every process, in workspace order.
    pub fn process_names(&self) -> Vec<ProcessName> {
        self.workspace
            .processes()
            .iter()
            .map(|process| process.name().clone())
            .collect()
    }

    /// The pid of the supervised child of the process named `name`, while it
    /// has a live handle.
    pub fn live_pid(&self, name: &ProcessName) -> Option<u32> {
        self.pane_named(name)
            .and_then(|pane| self.panes.get(&pane))
            .and_then(|target| target.handle.as_ref())
            .and_then(|handle| handle.process_id())
    }

    /// The pane of the process named `name`, if the project has one.
    pub(super) fn pane_named(&self, name: &ProcessName) -> Option<PaneId> {
        self.workspace
            .processes()
            .iter()
            .find(|process| process.name() == name)
            .map(|process| *process.id())
    }

    /// The pane of the process named `name`, reporting an unknown name as a
    /// notice.
    pub(super) fn resolve(&self, name: &ProcessName) -> Option<PaneId> {
        let pane = self.pane_named(name);
        if pane.is_none() {
            self.notify_process(
                NoticeLevel::Error,
                Some(name.clone()),
                UNKNOWN_PROCESS_NOTICE.to_string(),
            );
        }
        pane
    }

    /// Records a notice about `pane`'s process, or the project as a whole.
    pub(super) fn notify(
        &self,
        level: NoticeLevel,
        pane: Option<PaneId>,
        message: impl Into<String>,
    ) {
        let process = pane
            .and_then(|pane| self.workspace.process(pane))
            .map(|process| process.name().clone());
        self.notify_process(level, process, message.into());
    }

    /// Records a notice about the process named `process`.
    pub(super) fn notify_process(
        &self,
        level: NoticeLevel,
        process: Option<ProcessName>,
        message: String,
    ) {
        if level == NoticeLevel::Error
            && let (Some(name), Some(refusals)) =
                (process.as_ref(), self.batch_refusals.borrow_mut().as_mut())
        {
            refusals.push(
                Refusal::builder()
                    .name(name.clone())
                    .reason(message.clone())
                    .build(),
            );
        }
        let notice = Notice::builder()
            .at(SystemTime::now())
            .project(Some(self.project.clone()))
            .process(process)
            .level(level)
            .message(message)
            .build();
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notice);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossbeam_channel::unbounded;

    use super::*;
    use crate::{
        adapter::service::{Notices, supervisor::testing::*},
        domain::{
            config::{ProcessSpec, WorkspaceConfig},
            process::{Process, ProcessKind, ProcessState, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::{Cols, CommandLine, Rows},
        },
    };

    const PANE: u64 = 0;

    /// The name of the test process.
    fn name() -> ProcessName {
        ProcessName::try_new("p").unwrap()
    }

    /// `load` reads the project's config through the registry and starts
    /// nothing until asked.
    #[test]
    fn load_builds_the_workspace_from_the_projects_config() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![
                ProcessSpec::builder()
                    .name(name())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .autostart(true)
                    .build(),
            ])
            .commands(vec![])
            .build();
        let deps = SupervisorDeps::builder()
            .runner(Box::new(FakeRunner))
            .registry(Box::new(FakeRegistry {
                projects: Vec::new(),
                workspace: config,
                recorder: Recorder::default(),
            }))
            .agent_session_store(None)
            .events(unbounded().0)
            .notices(Notices::shared(None))
            .config_watcher(None)
            .notifier(None)
            .desktop_notifications(false)
            .process_metrics(None)
            .size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .color_scheme(ColorScheme::Dark)
            .build();

        let supervisor =
            Supervisor::load(ProjectKey::new(PathBuf::from(TEST_PROJECT)), deps).unwrap();

        let status = supervisor.snapshot();
        assert_eq!(status.processes().len(), 1);
        assert_eq!(status.processes()[0].name(), &name());
        assert_eq!(*status.processes()[0].state(), ProcessState::Pending);
    }

    /// One terminal named `p` on [`PANE`] with `restart` policy.
    fn terminal(restart: RestartPolicy) -> Process {
        Process::builder()
            .id(PaneId::new(PANE))
            .name(name())
            .kind(ProcessKind::Terminal)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .restart(restart)
            .build()
    }

    /// `ps` reports a restarted process's restarts, its uptime and its attached
    /// clients.
    #[test]
    fn the_snapshot_reports_restarts_uptime_and_attached_clients() {
        let mut harness = Harness::new(Box::new(FakeRunner), vec![terminal(RestartPolicy::Always)]);
        harness.supervisor.up();
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        let respawn = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_respawn(pane, respawn);
        harness.supervisor.set_attached(&name(), 2);

        let status = harness.supervisor.snapshot();

        let process = &status.processes()[0];
        assert_eq!(status.project(), harness.supervisor.project());
        assert_eq!(*process.state(), ProcessState::Running);
        assert_eq!(*process.restarts(), 1);
        assert!(process.uptime().is_some());
        assert_eq!(*process.attached(), 2);
    }
}
