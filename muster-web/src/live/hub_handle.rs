use crossbeam_channel::{Sender, bounded};

use super::{hub_event::HubEvent, hub_gone::HubGone, subscription::Subscription};
use crate::floor::{LocatedLog, LogWatch};

/// How every other thread reaches the hub: it only sends it events, and
/// waits on an answer only for an ask.
#[derive(Clone, Debug)]
pub struct HubHandle {
    /// Where the events go.
    events: Sender<HubEvent>,
}

impl HubHandle {
    /// A handle sending to `events`.
    pub(super) fn new(events: Sender<HubEvent>) -> Self {
        Self { events }
    }

    /// Sends `event`; false once the hub is gone.
    pub fn send(&self, event: HubEvent) -> bool {
        self.events.send(event).is_ok()
    }

    /// A stream for a page, its snapshot first; none when too many streams
    /// are open.
    ///
    /// # Errors
    /// Returns [`HubGone`] once the hub has ended.
    pub fn subscribe(&self) -> Result<Option<Subscription>, HubGone> {
        self.ask(HubEvent::Subscribe)
    }

    /// The log of the process named `name` in the project named `project`,
    /// as a request wrote them; none unless it is on the floor.
    ///
    /// # Errors
    /// Returns [`HubGone`] once the hub has ended.
    pub fn locate_log(&self, project: &str, name: &str) -> Result<Option<LocatedLog>, HubGone> {
        self.ask(|reply| HubEvent::LocateLog {
            project: project.to_string(),
            name: name.to_string(),
            reply,
        })
    }

    /// The running processes whose logs to stat.
    ///
    /// # Errors
    /// Returns [`HubGone`] once the hub has ended.
    pub fn log_watches(&self) -> Result<Vec<LogWatch>, HubGone> {
        self.ask(HubEvent::LogWatches)
    }

    /// Tells the hub the view is ending.
    pub fn stop(&self) {
        let _ = self.events.send(HubEvent::Stop);
    }

    /// Sends the ask `event` makes of a reply channel and waits for its
    /// answer, which the hub gives as soon as it reaches the ask.
    ///
    /// # Errors
    /// Returns [`HubGone`] when the hub ended before answering.
    fn ask<T>(&self, event: impl FnOnce(Sender<T>) -> HubEvent) -> Result<T, HubGone> {
        let (reply, answer) = bounded(1);
        self.events.send(event(reply)).map_err(|_| HubGone)?;
        answer.recv().map_err(|_| HubGone)
    }
}
