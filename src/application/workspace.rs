use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    agent_session::AgentSessionId,
    process::{ActivityState, Process, ProcessState},
    pty::ExitOutcome,
    value::PaneId,
};

/// In-memory workspace state: the ordered processes. Pure domain
/// orchestration, free of any rendering or I/O concern.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Workspace {
    processes: Vec<Process>,
}

impl Workspace {
    /// Whether the workspace has no processes.
    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    /// Inserts a process at the end of its section and returns its index.
    pub fn insert_in_section(&mut self, process: Process) -> usize {
        let section = process.kind().section_index();
        let index = self
            .processes
            .iter()
            .position(|existing| existing.kind().section_index() > section)
            .unwrap_or(self.processes.len());
        self.processes.insert(index, process);
        index
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

    /// Sets the autostart flag of the process owning `pane`.
    pub fn set_autostart(&mut self, pane: PaneId, autostart: bool) {
        if let Some(index) = self.position_of(pane) {
            self.processes[index].set_autostart(autostart);
        }
    }

    /// Updates the inferred activity of the process owning `pane`.
    pub fn set_activity(&mut self, pane: PaneId, activity: ActivityState) {
        if let Some(index) = self.position_of(pane) {
            self.processes[index].set_activity(activity);
        }
    }

    /// Binds the process owning `pane` to a durable agent session.
    pub fn set_agent_session_id(&mut self, pane: PaneId, id: AgentSessionId) {
        if let Some(index) = self.position_of(pane) {
            self.processes[index].set_agent_session_id(id);
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
    use super::*;
    use crate::domain::{
        process::{ProcessKind, RestartPolicy},
        value::{CommandLine, ProcessName},
    };

    fn process(id: u64, restart: RestartPolicy) -> Process {
        Process::builder()
            .id(PaneId::new(id))
            .name(ProcessName::try_new(format!("p{id}")).unwrap())
            .kind(ProcessKind::Command)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .restart(restart)
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

    /// A runtime process joins its kind's existing section.
    #[test]
    fn inserts_a_process_at_the_end_of_its_section() {
        let mut ws = workspace();
        let agent = Process::builder()
            .id(PaneId::new(2))
            .name(ProcessName::try_new("agent").unwrap())
            .kind(ProcessKind::Agent)
            .build();

        let index = ws.insert_in_section(agent);

        assert_eq!(index, 0);
        assert_eq!(*ws.processes()[index].kind(), ProcessKind::Agent);
    }
}
