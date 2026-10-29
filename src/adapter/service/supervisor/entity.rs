use std::collections::{HashMap, HashSet};

use crossbeam_channel::Sender;
use getset::{Getters, MutGetters};

use super::{
    adopter::HolderAdopter, adoption_retries::AdoptionRetries, attach_book::AttachBook,
    attention::Attention, deps::SupervisorDeps, held_wait::HeldWait, launch_book::LaunchBook,
    metrics_book::MetricsBook, pane::Pane, restart_book::RestartBook, run_readiness::RunReadiness,
    spawn_generation::SpawnGeneration,
};
use crate::{
    adapter::service::{OutputTap, ProbeWorker, ServiceEvent, StoreRecorder, WaitGroup, Waiter},
    application::{StackSync, Workspace},
    domain::{
        config::Stack,
        port::ProcessRunner,
        project::Project,
        settings::Settings,
        value::{PaneId, ProjectRoot},
    },
};

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
    /// The retries of adoptions that failed while their child runs on.
    pub(super) adoption_retries: AdoptionRetries,
    /// Where output, restart timers and stop deadlines are delivered.
    pub(super) events: Sender<ServiceEvent>,
    /// The last screen and live handle of every pane that has run.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    pub(super) panes: HashMap<PaneId, Pane>,
    /// The current spawn generation of every pane, so stale events are dropped.
    #[getset(get = "pub(crate)")]
    pub(super) generations: HashMap<PaneId, SpawnGeneration>,
    /// Each pane's restarts, backoff, last exit and failure streaks.
    pub(super) restarts: RestartBook,
    /// Launches and adoptions in flight, and the environment launches run
    /// with.
    pub(super) launches: LaunchBook,
    /// Notices, the event log, desktop alerts and what decides they are told.
    pub(super) attention: Attention,
    /// Sampled usage and ports, and the starts a held port blocked.
    pub(super) metrics: MetricsBook,
    /// Attached clients and exits held for a pending restart.
    pub(super) attach: AttachBook,
    /// The settings launches take their terminal, log and colors from, and
    /// that decide whether a crash reaches the desktop.
    pub(super) settings: Settings,
    /// Tries port and HTTP probes off the loop.
    pub(super) probes: Option<ProbeWorker>,
    /// Each pane's output as lines, across its runs.
    pub(super) taps: HashMap<PaneId, OutputTap>,
    /// Waits not yet answered.
    pub(super) waiters: Vec<Waiter>,
    /// Waits held, by pane, while the log's end of a process this service
    /// never saw run is read.
    pub(super) held_waits: HashMap<PaneId, Vec<HeldWait>>,
    /// The id the next held wait takes, so its timeout finds it.
    pub(super) next_held_wait: u64,
    /// Waits of a project-wide start, answered together, by group.
    pub(super) wait_groups: HashMap<u64, WaitGroup>,
    /// The id the next group of waits takes.
    pub(super) next_wait_group: u64,
    /// The readiness of each pane's current run that has a check.
    pub(super) readiness: HashMap<PaneId, RunReadiness>,
    /// Stores how each run ends, so a later service shows it; absent where
    /// nothing is stored.
    pub(super) runs: Option<StoreRecorder>,
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
            runs,
        } = deps;
        let empty = Workspace::builder().processes(Vec::new()).build();
        Self {
            project,
            workspace: StackSync::apply(&empty, stack.processes(), |_| false).into_workspace(),
            runner,
            adopter: None,
            adoption_retries: AdoptionRetries::default(),
            events,
            panes: HashMap::new(),
            generations: HashMap::new(),
            restarts: RestartBook::default(),
            launches: LaunchBook::default(),
            attention: Attention::new(notices, event_log, notifier),
            metrics: MetricsBook::new(port_occupants),
            attach: AttachBook::default(),
            settings,
            probes,
            taps: HashMap::new(),
            waiters: Vec::new(),
            held_waits: HashMap::new(),
            next_held_wait: 0,
            wait_groups: HashMap::new(),
            next_wait_group: 0,
            readiness: HashMap::new(),
            runs,
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

    /// Lets go of every process without stopping it: each handle is dropped,
    /// so a holder keeps its process running for the next service to adopt.
    /// Every generation advances, including that of a process with a restart
    /// pending but no terminal, so a timer still in flight is stale. A launch
    /// or adoption in flight is not cancelled: its result, once it comes back
    /// (see [`Self::launches_in_flight`]), is detached the same way. An exit
    /// held for a pending restart is left unacknowledged, so the next service
    /// applies it again, unless its respawn is in flight and settles it.
    pub fn shutdown(&mut self) {
        self.launches.detach_all();
        for pane in self.panes.values_mut() {
            pane.take_handle();
        }
        let respawning: HashSet<PaneId> = self
            .attach
            .holding()
            .into_iter()
            .filter(|pane| self.is_spawning(*pane))
            .collect();
        self.attach.keep_held(|pane| respawning.contains(&pane));
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
}
