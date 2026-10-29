use super::entity::Supervisor;
use crate::{
    adapter::service::NoticeLevel,
    domain::value::{PaneId, Pid, ProcessName},
};

/// Shown when a process name matches nothing in the project.
pub(super) const UNKNOWN_PROCESS_NOTICE: &str = "no process with that name in this project";

impl Supervisor {
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
            .and_then(|target| target.handle().as_ref())
            .and_then(|handle| handle.process_id())
    }

    /// Records how many writable clients are attached to `name`'s terminal.
    #[cfg(test)]
    pub fn set_attached(&mut self, name: &ProcessName, writable_clients: u32) {
        if let Some(pane) = self.pane_named(name) {
            self.attach.set_attached(pane, writable_clients);
        }
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
        self.attention.notify(self.root(), level, process, message);
    }
}

#[cfg(test)]
mod tests {
    use crate::{adapter::service::supervisor::testing::*, domain::process::RestartPolicy};

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
