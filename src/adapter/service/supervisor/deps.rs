use crossbeam_channel::Sender;
use typed_builder::TypedBuilder;

use crate::{
    adapter::service::{ProbeWorker, ServiceEvent, SharedEventLog, SharedNotices},
    domain::{
        port::{Notifier, ProcessRunner},
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
    /// Where lifecycle events are recorded for clients to read and wait on.
    pub(super) event_log: SharedEventLog,
    /// Tries port and HTTP probes off the loop; absent where none run.
    pub(super) probes: Option<ProbeWorker>,
    /// Delivers desktop notifications; absent where none are shown.
    pub(super) notifier: Option<Box<dyn Notifier>>,
    /// The settings each launch takes its terminal, log and colors from, and
    /// that decide whether a crash reaches the desktop.
    pub(super) settings: Settings,
}
