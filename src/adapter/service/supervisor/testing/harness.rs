use crossbeam_channel::{Receiver, unbounded};

use super::fixtures::{stack, test_project, test_settings};
use crate::{
    adapter::service::{
        EventLog, LaunchEnvironment, Notices, ServiceEvent, SharedEventLog, SharedNotices,
        supervisor::{Supervisor, SupervisorDeps},
    },
    application::Workspace,
    domain::{config::ProcessSpec, port::ProcessRunner, process::Process, value::PaneId},
};

/// A supervisor under test, with the channel its timers and output arrive on
/// and the notices it raises.
pub(crate) struct Harness {
    pub(crate) supervisor: Supervisor,
    pub(crate) events: Receiver<ServiceEvent>,
    pub(crate) notices: SharedNotices,
    pub(crate) event_log: SharedEventLog,
}

impl Harness {
    /// A supervisor of [`TEST_PROJECT`](super::TEST_PROJECT) running a stack
    /// of `processes`, spawning through `runner`, with nothing started yet.
    pub(crate) fn over(runner: Box<dyn ProcessRunner>, processes: &[ProcessSpec]) -> Self {
        let (sender, events) = unbounded();
        let notices = Notices::shared(None);
        let event_log = EventLog::shared();
        let deps = SupervisorDeps::builder()
            .runner(runner)
            .events(sender)
            .notices(notices.clone())
            .event_log(event_log.clone())
            .probes(None)
            .notifier(None)
            .port_occupants(None)
            .settings(test_settings())
            .build();
        Self {
            supervisor: Supervisor::new(test_project(), &stack(processes), deps),
            events,
            notices,
            event_log,
        }
    }

    /// A supervisor of [`TEST_PROJECT`](super::TEST_PROJECT) whose workspace
    /// is exactly `processes`, panes and states as given, spawning through
    /// `runner`.
    pub(crate) fn new(runner: Box<dyn ProcessRunner>, processes: Vec<Process>) -> Self {
        let mut harness = Self::over(runner, &[]);
        *harness.supervisor.workspace_mut() = Workspace::builder().processes(processes).build();
        harness
    }

    /// Brings the project up as a client with no environment of its own would.
    pub(crate) fn up(&mut self) {
        self.supervisor.up(&LaunchEnvironment::default());
    }

    /// The messages of every notice raised so far, oldest first.
    pub(crate) fn notice_messages(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap()
            .recent(self.supervisor.root())
            .iter()
            .map(|notice| notice.message().clone())
            .collect()
    }

    /// The lines of `pane`'s current run its output tap holds, oldest first.
    pub(crate) fn run_tail(&self, pane: PaneId) -> Vec<String> {
        self.supervisor
            .taps
            .get(&pane)
            .map(|tap| tap.tail(usize::MAX))
            .unwrap_or_default()
    }
}
