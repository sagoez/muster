use super::{config_membership::ConfigMembership, entity::Supervisor};
use crate::{application::StackSync, domain::config::Stack};

impl Supervisor {
    /// Brings the workspace in line with `stack`, the project's stored stack,
    /// without bouncing anything it keeps: a kept process takes its new
    /// definition in place and keeps running, so its next start uses it; a
    /// new one waits in `Pending`; a removed one that may still run is
    /// stopped through its stop policy and retires once it exits, and one that
    /// cannot run goes at once. A process whose name comes back while it
    /// retires is kept again.
    pub fn apply_stack(&mut self, stack: &Stack) {
        let sync = StackSync::apply(&self.workspace, stack.processes(), |pane| {
            self.holds_process(pane)
        });
        let added = sync.added().clone();
        let retiring = sync.retiring().clone();
        let removed = sync.removed().clone();
        self.workspace = sync.into_workspace();
        for pane in removed {
            self.retire_pane(pane);
        }
        let kept: Vec<_> = self
            .workspace
            .processes()
            .iter()
            .map(|process| *process.id())
            .filter(|pane| !added.contains(pane) && !retiring.contains(pane))
            .collect();
        for pane in kept {
            self.set_membership(pane, ConfigMembership::Tracked);
        }
        for pane in retiring {
            self.set_membership(pane, ConfigMembership::RetireOnExit);
            self.stop_pane(pane);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::{
        adapter::service::{LaunchEnvironment, supervisor::testing::*},
        domain::{
            process::{ProcessState, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::PaneId,
        },
    };

    /// The state of the process named `raw`.
    fn state_of(harness: &Harness, raw: &str) -> ProcessState {
        let key = name(raw).key();
        *harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .find(|process| process.name().key() == key)
            .unwrap()
            .state()
    }

    /// The names of the workspace's processes, in order.
    fn names(harness: &Harness) -> Vec<String> {
        harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .map(|process| process.name().to_string())
            .collect()
    }

    /// Delivers `outcome` as the exit of the process on `pane`.
    fn exit(harness: &mut Harness, pane: PaneId, outcome: ExitOutcome) {
        let generation = harness.supervisor.generations()[&pane];
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Exited(outcome));
    }

    /// A stack gaining a process adds it in `Pending` and starts nothing.
    #[test]
    fn an_added_process_waits_pending() {
        let spawns = SpawnRecorder::default();
        let mut harness = Harness::over(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            &[spec("web", "npm run dev", RestartPolicy::Never)],
        );
        harness.up();

        harness.supervisor.apply_stack(&stack(&[
            spec("web", "npm run dev", RestartPolicy::Never),
            spec("api", "cargo run", RestartPolicy::Never),
        ]));

        assert_eq!(names(&harness), ["web", "api"]);
        assert_eq!(state_of(&harness, "api"), ProcessState::Pending);
        assert_eq!(spawns.requests.borrow().len(), 1, "nothing new started");
    }

    /// A changed running process keeps running unrestarted; its next start
    /// uses the new definition.
    #[test]
    fn a_changed_running_process_keeps_running_and_restarts_with_the_change() {
        let spawns = SpawnRecorder::default();
        let mut harness = Harness::over(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            &[spec("web", "npm run dev", RestartPolicy::Never)],
        );
        harness.up();
        let generation = harness.supervisor.generations()[&PaneId::new(0)];

        harness
            .supervisor
            .apply_stack(&stack(&[spec("web", "pnpm dev", RestartPolicy::Never)]));

        assert_eq!(state_of(&harness, "web"), ProcessState::Running);
        assert_eq!(
            harness.supervisor.generations()[&PaneId::new(0)],
            generation,
            "not restarted"
        );
        harness
            .supervisor
            .restart(&name("web"), &LaunchEnvironment::default());
        exit(&mut harness, PaneId::new(0), ExitOutcome::Succeeded);
        let requests = spawns.requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].command().as_ref(), "pnpm dev");
    }

    /// A removed running process is stopped through its stop policy and its
    /// row goes once it exits; a removed stopped one goes at once.
    #[test]
    fn a_removed_running_process_is_stopped_and_discarded_on_exit() {
        let signals = Arc::new(StopSignals::default());
        let mut harness = Harness::over(
            Box::new(StopSignalRunner {
                signals: Arc::clone(&signals),
            }),
            &[
                spec("web", "npm run dev", RestartPolicy::Always),
                spec("idle", "true", RestartPolicy::Never).with_autostart(false),
            ],
        );
        harness.up();

        harness.supervisor.apply_stack(&stack(&[]));

        assert_eq!(names(&harness), ["web"], "the stopped one went at once");
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(state_of(&harness, "web"), ProcessState::Stopping);
        exit(&mut harness, PaneId::new(0), ExitOutcome::FAILED_UNKNOWN);
        assert!(harness.supervisor.workspace().is_empty());
        assert!(harness.supervisor.is_idle());
    }

    /// A name added back while its process retires is kept: its exit settles
    /// it instead of retiring the row.
    #[test]
    fn a_name_added_back_while_retiring_is_kept() {
        let kills = Arc::new(AtomicUsize::new(0));
        let mut harness = Harness::over(
            Box::new(KillCountRunner {
                kills: Arc::clone(&kills),
            }),
            &[spec("web", "npm run dev", RestartPolicy::Never)],
        );
        harness.up();
        harness.supervisor.apply_stack(&stack(&[]));

        harness
            .supervisor
            .apply_stack(&stack(&[spec("web", "npm run dev", RestartPolicy::Never)]));
        exit(&mut harness, PaneId::new(0), ExitOutcome::Succeeded);

        assert_eq!(names(&harness), ["web"]);
        assert_eq!(state_of(&harness, "web"), ProcessState::Exited);
    }

    /// A removed process left running unsupervised is stopped by signal and
    /// its row goes once it is gone.
    #[test]
    fn a_removed_unsupervised_process_is_stopped_and_discarded() {
        let orphan = Orphan::spawn();
        let mut harness = Harness::over(Box::new(FakeRunner), &[spec(
            "web",
            "npm run dev",
            RestartPolicy::Never,
        )]);
        harness.supervisor.advance_generation(PaneId::new(0));
        assert!(
            harness
                .supervisor
                .leave_unsupervised(PaneId::new(0), orphan.pid(), None)
        );

        harness.supervisor.apply_stack(&stack(&[]));
        assert_eq!(state_of(&harness, "web"), ProcessState::Stopping);
        assert!(orphan.end_with(libc::SIGKILL).is_some(), "the orphan ended");
        let pane = PaneId::new(0);
        let spawn = harness.supervisor.generations()[&pane];
        let shutdown = harness.supervisor.panes()[&pane].shutdown_generation;
        harness.supervisor.handle_force_stop(pane, spawn, shutdown);

        assert!(harness.supervisor.workspace().is_empty());
    }
}
