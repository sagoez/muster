use crossbeam_channel::Sender;
use typed_builder::TypedBuilder;

use crate::{
    adapter::service::{ServiceEvent, SharedNotices},
    domain::{
        port::{Notifier, ProcessMetrics, ProcessRunner},
        settings::Settings,
    },
};

/// The driven adapters and channels one [`Supervisor`](super::Supervisor) runs
/// on. No `Getters`: the boxed ports are moved out once, not borrowed.
#[derive(TypedBuilder)]
pub struct SupervisorDeps {
    /// Spawns each process.
    pub(super) runner: Box<dyn ProcessRunner>,
    /// Where process output, restart timers and stop deadlines are delivered.
    pub(super) events: Sender<ServiceEvent>,
    /// Where supervision reports what the user should know.
    pub(super) notices: SharedNotices,
    /// Delivers desktop notifications; absent where none are shown.
    pub(super) notifier: Option<Box<dyn Notifier>>,
    /// Samples each running process's resource usage; absent where unread.
    pub(super) process_metrics: Option<Box<dyn ProcessMetrics>>,
    /// The settings each launch takes its terminal, log and colors from, and
    /// that decide whether a crash reaches the desktop.
    pub(super) settings: Settings,
}
