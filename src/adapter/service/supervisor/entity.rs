use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ffi::OsString,
    sync::PoisonError,
    time::SystemTime,
};

use crossbeam_channel::Sender;
use getset::{Getters, MutGetters};

use super::{
    adopter::HolderAdopter, config_membership::ConfigMembership, deps::SupervisorDeps, pane::Pane,
    pending_launch::PendingLaunch, run_end::RunEnd, run_readiness::RunReadiness,
    spawn_generation::SpawnGeneration,
};
use crate::{
    adapter::service::{
        EventDraft, EventKind, ExitReason, LastExit, Notice, NoticeLevel, OutputTap, PortBlock,
        PortHolder, ProbeWorker, ProcessStatus, ProcessUsage, ProjectStatus, Refusal, ServiceEvent,
        SharedEventLog, SharedNotices, StackUrl, UrlSource, WaitGroup, Waiter,
    },
    application::{StackSync, Workspace},
    domain::{
        config::Stack,
        port::{ListeningSocket, Notifier, PortOccupants, ProcessHandle, ProcessRunner},
        process::{ProcessState, ReadyCheck, ReadyProbe},
        project::Project,
        pty::ExitOutcome,
        settings::Settings,
        value::{PaneId, Pid, ProcessName, ProjectRoot},
    },
};

/// Shown when a process name matches nothing in the project.
pub(super) const UNKNOWN_PROCESS_NOTICE: &str = "no process with that name in this project";
/// What a detected port's URL starts with, before the port.
const DETECTED_URL_PREFIX: &str = "http://localhost:";

/// Supervises one project's processes: spawns them, applies their stop and
/// restart policies, adopts holders a previous service left running, follows
/// the project's stored stack, and reports failures as notices rather than
/// panicking.
#[derive(Getters, MutGetters)]
pub struct Supervisor {
    /// The project this supervisor runs.
    #[getset(get = "pub")]
    pub(super) project: Project,
    /// The project's processes and their lifecycle state.
    #[getset(get = "pub")]
    pub(super) workspace: Workspace,
    /// Spawns each process.
    pub(super) runner: Box<dyn ProcessRunner>,
    /// Takes over holders a previous service left running.
    pub(super) adopter: Option<Box<dyn HolderAdopter>>,
    /// Where output, restart timers and stop deadlines are delivered.
    pub(super) events: Sender<ServiceEvent>,
    /// Where supervision reports what the user should know.
    pub(super) notices: SharedNotices,
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
    /// Delivers desktop notifications.
    pub(super) notifier: Option<Box<dyn Notifier>>,
    /// Finds who listens on a port a start needs, when no supervised
    /// process was seen on it.
    pub(super) port_occupants: Option<Box<dyn PortOccupants>>,
    /// The settings launches take their terminal, log and colors from, and
    /// that decide whether a crash reaches the desktop.
    pub(super) settings: Settings,
    /// Panes whose last holder screen read failed, so a failure streak is
    /// reported once.
    pub(super) failing_screen_reads: HashSet<PaneId>,
    /// Writable clients attached to each live pane's terminal.
    pub(super) attached: HashMap<PaneId, u32>,
    /// The handle of each pane's exited process whose exit is not yet
    /// acknowledged, held while its restart is pending so a service that
    /// stops meanwhile leaves the exit for its successor to apply again.
    pub(super) held_exits: HashMap<PaneId, Box<dyn ProcessHandle>>,
    /// The environment of the command that last started each pane, which its
    /// policy restarts are launched with too.
    pub(super) launch_environments: HashMap<PaneId, BTreeMap<OsString, OsString>>,
    /// Where lifecycle events are recorded.
    pub(super) event_log: SharedEventLog,
    /// Tries port and HTTP probes off the loop.
    pub(super) probes: Option<ProbeWorker>,
    /// Each pane's output as lines, across its runs.
    pub(super) taps: HashMap<PaneId, OutputTap>,
    /// Waits not yet answered.
    pub(super) waiters: Vec<Waiter>,
    /// Waits of a project-wide start, answered together, by group.
    pub(super) wait_groups: HashMap<u64, WaitGroup>,
    /// The id the next group of waits takes.
    pub(super) next_wait_group: u64,
    /// The readiness of each pane's current run that has a check.
    pub(super) readiness: HashMap<PaneId, RunReadiness>,
    /// How each pane's most recent run ended, while this service saw it.
    pub(super) run_ends: HashMap<PaneId, RunEnd>,
    /// The ports each live pane's process tree was last seen listening on.
    pub(super) ports: HashMap<PaneId, Vec<ListeningSocket>>,
    /// Every port a supervised process of any project listens on, and who
    /// holds it, as the service last shared it.
    pub(super) held_ports: HashMap<u16, PortHolder>,
    /// Each pane whose last start was refused for a held port, and why.
    pub(super) blocks: HashMap<PaneId, PortBlock>,
}

impl Supervisor {
    /// Creates the supervisor of `project` running `stack`. Nothing starts
    /// until [`Self::up`] or a start request.
    pub fn new(project: Project, stack: &Stack, deps: SupervisorDeps) -> Self {
        let SupervisorDeps {
            runner,
            events,
            notices,
            event_log,
            probes,
            notifier,
            port_occupants,
            settings,
        } = deps;
        let empty = Workspace::builder().processes(Vec::new()).build();
        Self {
            project,
            workspace: StackSync::apply(&empty, stack.processes(), |_| false).into_workspace(),
            runner,
            adopter: None,
            events,
            notices,
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
            notifier,
            port_occupants,
            settings,
            failing_screen_reads: HashSet::new(),
            attached: HashMap::new(),
            held_exits: HashMap::new(),
            launch_environments: HashMap::new(),
            event_log,
            probes,
            taps: HashMap::new(),
            waiters: Vec::new(),
            wait_groups: HashMap::new(),
            next_wait_group: 0,
            readiness: HashMap::new(),
            run_ends: HashMap::new(),
            ports: HashMap::new(),
            held_ports: HashMap::new(),
            blocks: HashMap::new(),
        }
    }

    /// Lets this supervisor take over holders a previous service left running.
    pub fn with_adopter(mut self, adopter: Box<dyn HolderAdopter>) -> Self {
        self.adopter = Some(adopter);
        self
    }

    /// The folder of the project this supervisor runs.
    pub fn root(&self) -> &ProjectRoot {
        self.project.root()
    }

    /// Takes `settings` for every later launch and for crash alerts; running
    /// processes keep the terminal they started with.
    pub fn apply_settings(&mut self, settings: &Settings) {
        self.settings = settings.clone();
    }

    /// Swaps in desktop notification delivery, for a test to inject a fake.
    #[cfg(test)]
    pub(crate) fn set_notifier(&mut self, notifier: Box<dyn Notifier>) {
        self.notifier = Some(notifier);
    }

    /// Swaps in the lookup of who listens on a port, for a test to inject a
    /// fake.
    #[cfg(test)]
    pub(crate) fn set_port_occupants(&mut self, occupants: Box<dyn PortOccupants>) {
        self.port_occupants = Some(occupants);
    }

    /// The project's processes, for a test to arrange state directly.
    #[cfg(test)]
    pub(crate) fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    /// Whether this supervisor runs nothing and has nothing in flight, so a
    /// project no longer registered can be let go.
    pub fn is_idle(&self) -> bool {
        self.workspace.is_empty() && self.launches.is_empty()
    }

    /// A point-in-time view of every process, for `ps`. An unsupervised child
    /// found gone is shown as it will settle: exited, or no row at all when its
    /// row was removed from the stack, so `ps` never shows an unsupervised
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
                let ports = self.ports.get(&pane).cloned().unwrap_or_default();
                Some(
                    ProcessStatus::builder()
                        .name(process.name().clone())
                        .state(state)
                        .ready(
                            self.readiness
                                .get(&pane)
                                .map(|readiness| readiness.state)
                                .unwrap_or_default(),
                        )
                        .urls(urls(process.ready().as_ref(), &ports))
                        .ports(ports)
                        .run_start(self.output_position(pane).1)
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
                                .then(|| self.last_exit(pane))
                                .flatten(),
                        )
                        .stop_grace(process.stop().grace_period().into_inner())
                        .blocked_on(
                            self.blocks
                                .get(&pane)
                                .filter(|_| !state.may_be_running())
                                .cloned(),
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

    /// How `pane`'s most recent run ended: why, as this service saw it, or as
    /// its outcome tells for one an earlier service saw.
    fn last_exit(&self, pane: PaneId) -> Option<LastExit> {
        let end = self.run_ends.get(&pane);
        let outcome = match end {
            Some(end) if end.reason == ExitReason::FailedToStart => ExitOutcome::FAILED_UNKNOWN,
            _ => self.last_exits.get(&pane)?.clone(),
        };
        Some(
            LastExit::builder()
                .reason(end.map_or_else(|| ExitReason::of(&outcome, false), |end| end.reason))
                .outcome(outcome)
                .at(end.map(|end| end.at))
                .build(),
        )
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
    pub fn live_pid(&self, name: &ProcessName) -> Option<Pid> {
        self.pane_named(name)
            .and_then(|pane| self.panes.get(&pane))
            .and_then(|target| target.handle.as_ref())
            .and_then(|handle| handle.process_id())
    }

    /// The pane of the process named `name`, matched by name key, if the
    /// project has one.
    pub(super) fn pane_named(&self, name: &ProcessName) -> Option<PaneId> {
        let key = name.key();
        self.workspace
            .processes()
            .iter()
            .find(|process| process.name().key() == key)
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
        self.event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(
                EventDraft::builder()
                    .project(Some(self.root().clone()))
                    .process(process.clone())
                    .kind(EventKind::Notice)
                    .detail(message.clone())
                    .build(),
            );
        let notice = Notice::builder()
            .at(SystemTime::now())
            .project(Some(self.root().clone()))
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

/// The URLs a process serves: the one its readiness check declares, else
/// `http://localhost:<port>` for each port it listens on.
fn urls(check: Option<&ReadyCheck>, ports: &[ListeningSocket]) -> Vec<StackUrl> {
    if let Some(ReadyProbe::Http(url)) = check.map(ReadyCheck::probe) {
        return vec![
            StackUrl::builder()
                .url(url.to_string())
                .source(UrlSource::Declared)
                .build(),
        ];
    }
    let distinct: BTreeSet<u16> = ports.iter().map(ListeningSocket::port).collect();
    distinct
        .into_iter()
        .map(|port| {
            StackUrl::builder()
                .url(format!("{DETECTED_URL_PREFIX}{port}"))
                .source(UrlSource::Detected)
                .build()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            process::{ProcessState, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
        },
    };

    const PANE: u64 = 0;

    /// `new` builds the workspace from the stack and starts nothing.
    #[test]
    fn a_new_supervisor_runs_the_stack_and_starts_nothing() {
        let harness = Harness::over(Box::new(FakeRunner), &[spec(
            "p",
            "true",
            RestartPolicy::Never,
        )]);

        let status = harness.supervisor.snapshot();

        assert_eq!(status.project(), harness.supervisor.project());
        assert_eq!(status.processes().len(), 1);
        assert_eq!(status.processes()[0].name(), &name("p"));
        assert_eq!(*status.processes()[0].state(), ProcessState::Pending);
        assert_eq!(*status.processes()[0].stop_grace(), TEST_STOP_GRACE);
    }

    /// `ps` reports a restarted process's restarts, its uptime and its attached
    /// clients.
    #[test]
    fn the_snapshot_reports_restarts_uptime_and_attached_clients() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[spec(
            "p",
            "true",
            RestartPolicy::Always,
        )]);
        harness.up();
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        let respawn = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_respawn(pane, respawn);
        harness.supervisor.set_attached(&name("p"), 2);

        let status = harness.supervisor.snapshot();

        let process = &status.processes()[0];
        assert_eq!(*process.state(), ProcessState::Running);
        assert_eq!(*process.restarts(), 1);
        assert!(process.uptime().is_some());
        assert_eq!(*process.attached(), 2);
    }

    /// A name is matched by its key, however its accent was encoded.
    #[test]
    fn a_process_is_found_by_its_name_key() {
        let harness = Harness::over(Box::new(FakeRunner), &[spec(
            "caf\u{e9}",
            "true",
            RestartPolicy::Never,
        )]);

        assert!(harness.supervisor.has_process(&name("cafe\u{301}")));
    }
}
