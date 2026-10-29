use crossbeam_channel::Sender;

use super::event::ServiceEvent;
use crate::domain::port::{
    AgentSessionStore, ConfigWatcher, Notifier, ProcessMetrics, ProjectRegistry,
};

/// Builds the driven adapters each project's supervisor runs on, as the
/// service loads that project. An adapter that is absent is not wired.
pub trait ProjectPorts {
    /// Reads and writes project configs.
    fn registry(&self) -> Box<dyn ProjectRegistry>;

    /// Durable agent-session history.
    fn agent_session_store(&self) -> Option<Box<dyn AgentSessionStore>>;

    /// Desktop notifications.
    fn notifier(&self) -> Option<Box<dyn Notifier>>;

    /// Resource usage sampling.
    fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>>;

    /// A config watcher delivering its changes on `events`.
    fn config_watcher(&self, events: Sender<ServiceEvent>) -> Option<Box<dyn ConfigWatcher>>;
}
