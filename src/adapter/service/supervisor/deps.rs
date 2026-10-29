use crossbeam_channel::Sender;
use typed_builder::TypedBuilder;

use crate::{
    adapter::service::{ServiceEvent, SharedNotices},
    domain::{
        color_scheme::ColorScheme,
        port::{
            AgentSessionStore, ConfigWatcher, Notifier, ProcessMetrics, ProcessRunner,
            ProjectRegistry,
        },
        pty::PtySize,
    },
};

/// The driven adapters and channels one [`Supervisor`](super::Supervisor) runs
/// on. No `Getters`: the boxed ports are moved out once, not borrowed.
#[derive(TypedBuilder)]
pub struct SupervisorDeps {
    /// Spawns each process.
    pub(super) runner: Box<dyn ProcessRunner>,
    /// Reads and locks the project's config.
    pub(super) registry: Box<dyn ProjectRegistry>,
    /// Durable agent-session history; absent in compositions that omit it.
    pub(super) agent_session_store: Option<Box<dyn AgentSessionStore>>,
    /// Where process output, restart timers and stop deadlines are delivered.
    pub(super) events: Sender<ServiceEvent>,
    /// Where supervision reports what the user should know.
    pub(super) notices: SharedNotices,
    /// Reports edits to the project's config; absent where nothing watches.
    pub(super) config_watcher: Option<Box<dyn ConfigWatcher>>,
    /// Delivers desktop notifications; absent where none are shown.
    pub(super) notifier: Option<Box<dyn Notifier>>,
    /// Whether attention a process raises also reaches the desktop.
    pub(super) desktop_notifications: bool,
    /// Samples each running process's resource usage; absent where unread.
    pub(super) process_metrics: Option<Box<dyn ProcessMetrics>>,
    /// Terminal size each new process starts with.
    pub(super) size: PtySize,
    /// The default colors each process's terminal reports when asked.
    pub(super) color_scheme: ColorScheme,
}
