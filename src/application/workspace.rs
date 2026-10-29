use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{Process, ProcessState},
    pty::ExitOutcome,
    value::PaneId,
};

/// In-memory workspace state: the ordered processes. Pure domain
/// orchestration, free of any rendering or I/O concern.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Workspace {
    processes: Vec<Process>,
}

impl Workspace {
    /// Whether the workspace has no processes.
    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    /// Appends a process after every existing one.
    pub fn push(&mut self, process: Process) {
        self.processes.push(process);
    }

    /// Removes the process owning `pane`, if present.
    pub fn remove(&mut self, pane: PaneId) {
        if let Some(index) = self.position_of(pane) {
            self.processes.remove(index);
        }
    }

    /// Index of the process owning `pane`, if present.
    pub fn position_of(&self, pane: PaneId) -> Option<usize> {
        self.processes
            .iter()
            .position(|process| *process.id() == pane)
    }

    /// The process owning `pane`, if present.
    pub fn process(&self, pane: PaneId) -> Option<&Process> {
        self.position_of(pane).map(|index| &self.processes[index])
    }

    /// Updates the lifecycle state of the process owning `pane`.
    pub fn set_state(&mut self, pane: PaneId, state: ProcessState) {
        if let Some(index) = self.position_of(pane) {
            self.processes[index].set_state(state);
        }
    }

    /// Whether the process owning `pane` should be restarted after `outcome`.
    pub fn should_restart(&self, pane: PaneId, outcome: &ExitOutcome) -> bool {
        self.process(pane)
            .map(|process| process.restart().should_restart(outcome))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, ProcessName, StopGrace},
    };

    fn process(id: u64, restart: RestartPolicy) -> Process {
        Process::builder()
            .id(PaneId::new(id))
            .name(ProcessName::try_new(format!("p{id}")).unwrap())
            .command(CommandLine::try_new("true").unwrap())
            .environment(Environment::empty())
            .restart(restart)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .autostart(false)
            .build()
    }

    fn workspace() -> Workspace {
        Workspace::builder()
            .processes(vec![
                process(0, RestartPolicy::Never),
                process(1, RestartPolicy::Always),
            ])
            .build()
    }

    #[test]
    fn set_state_updates_the_named_process() {
        let mut ws = workspace();
        ws.set_state(PaneId::new(1), ProcessState::Running);
        assert_eq!(
            *ws.process(PaneId::new(1)).unwrap().state(),
            ProcessState::Running
        );
    }

    /// Removing a process drops only that row, and an absent one is a no-op.
    #[test]
    fn remove_drops_only_the_named_process() {
        let mut ws = workspace();

        ws.remove(PaneId::new(0));
        ws.remove(PaneId::new(7));

        assert_eq!(ws.processes().len(), 1);
        assert_eq!(*ws.processes()[0].id(), PaneId::new(1));
    }

    #[test]
    fn restart_decision_follows_policy() {
        let ws = workspace();
        assert!(!ws.should_restart(PaneId::new(0), &ExitOutcome::FAILED_UNKNOWN));
        assert!(ws.should_restart(PaneId::new(1), &ExitOutcome::FAILED_UNKNOWN));
    }

    #[test]
    fn push_appends_after_every_process() {
        let mut ws = workspace();

        ws.push(process(2, RestartPolicy::Never));

        assert_eq!(ws.position_of(PaneId::new(2)), Some(2));
    }
}
