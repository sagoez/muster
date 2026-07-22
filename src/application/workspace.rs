use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{ActivityState, Process, ProcessState},
    pty::ExitOutcome,
    value::PaneId,
};

/// In-memory workspace state: the ordered processes and the current selection.
/// Pure domain orchestration, free of any rendering or I/O concern.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Workspace {
    processes: Vec<Process>,
    #[builder(default)]
    selected_index: usize,
}

impl Workspace {
    /// The currently selected process, if any.
    pub fn selected_process(&self) -> Option<&Process> {
        self.processes.get(self.selected_index)
    }

    /// Whether the workspace has no processes.
    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    /// Inserts a process at the end of its sidebar section and returns its index.
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

    /// Removes the process owning `pane`, preserving the selected process when
    /// another row is removed and otherwise selecting the adjacent row.
    pub fn remove(&mut self, pane: PaneId) {
        if let Some(index) = self.position_of(pane) {
            let selected = self.selected_process().map(|process| *process.id());
            self.processes.remove(index);
            self.selected_index = selected
                .filter(|selected| *selected != pane)
                .and_then(|selected| self.position_of(selected))
                .unwrap_or_else(|| index.min(self.processes.len().saturating_sub(1)));
        }
    }

    /// Moves the selection to the next process, wrapping around.
    pub fn select_next(&mut self) {
        if !self.processes.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.processes.len();
        }
    }

    /// Selects the process at `index`, clamped to the valid range. A no-op when
    /// the workspace is empty.
    pub fn select_at(&mut self, index: usize) {
        if !self.processes.is_empty() {
            self.selected_index = index.min(self.processes.len() - 1);
        }
    }

    /// Moves the selection to the previous process, wrapping around.
    pub fn select_previous(&mut self) {
        if !self.processes.is_empty() {
            let last = self.processes.len() - 1;
            self.selected_index = if self.selected_index == 0 {
                last
            } else {
                self.selected_index - 1
            };
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

    /// Whether the process owning `pane` should be restarted after `outcome`.
    pub fn should_restart(&self, pane: PaneId, outcome: ExitOutcome) -> bool {
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
    fn selection_wraps_in_both_directions() {
        let mut ws = workspace();
        assert_eq!(*ws.selected_index(), 0);
        ws.select_previous();
        assert_eq!(*ws.selected_index(), 1);
        ws.select_next();
        assert_eq!(*ws.selected_index(), 0);
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

    /// Removing an earlier row keeps selection on the same process identity.
    #[test]
    fn removing_an_earlier_process_preserves_selection() {
        let mut ws = Workspace::builder()
            .processes(vec![
                process(0, RestartPolicy::Never),
                process(1, RestartPolicy::Never),
                process(2, RestartPolicy::Never),
            ])
            .selected_index(2)
            .build();

        ws.remove(PaneId::new(0));

        assert_eq!(*ws.selected_process().unwrap().id(), PaneId::new(2));
        assert_eq!(*ws.selected_index(), 1);
    }

    /// Removing the selected row chooses the row that occupied its position,
    /// falling back to the preceding row when the last process was removed.
    #[test]
    fn removing_the_selected_process_selects_an_adjacent_process() {
        let mut ws = Workspace::builder()
            .processes(vec![
                process(0, RestartPolicy::Never),
                process(1, RestartPolicy::Never),
                process(2, RestartPolicy::Never),
            ])
            .selected_index(1)
            .build();

        ws.remove(PaneId::new(1));
        assert_eq!(*ws.selected_process().unwrap().id(), PaneId::new(2));

        ws.remove(PaneId::new(2));
        assert_eq!(*ws.selected_process().unwrap().id(), PaneId::new(0));
    }

    #[test]
    fn restart_decision_follows_policy() {
        let ws = workspace();
        assert!(!ws.should_restart(PaneId::new(0), ExitOutcome::Failed));
        assert!(ws.should_restart(PaneId::new(1), ExitOutcome::Failed));
    }

    /// A runtime process joins its kind's existing sidebar section.
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
