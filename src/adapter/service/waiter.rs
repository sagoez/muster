use std::time::Instant;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{run_pin::RunPin, wait_condition::WaitCondition, wait_reply::WaitReply};
use crate::{
    adapter::holder::LineMatcher,
    domain::value::{OutputCursor, PaneId, ProcessName},
};

/// A wait registered on the service loop, answered when its condition holds,
/// its run ends, or its deadline passes.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub struct Waiter {
    /// The pane of the process waited on.
    #[getset(get_copy = "pub")]
    pane: PaneId,
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// What it waits for, the default resolved.
    #[getset(get = "pub")]
    until: WaitCondition,
    /// The compiled pattern of a log condition.
    #[getset(get = "pub")]
    #[builder(default)]
    matcher: Option<LineMatcher>,
    /// The runs it is about.
    #[getset(get_copy = "pub")]
    pin: RunPin,
    /// When it was registered.
    #[getset(get_copy = "pub")]
    started: Instant,
    /// When it answers that it is still waiting.
    #[getset(get_copy = "pub")]
    deadline: Instant,
    /// Where its answer goes.
    #[getset(get = "pub")]
    reply: WaitReply,
    /// Where scanning began, when the asked cursor was older than the output
    /// held.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    scanned_from: Option<OutputCursor>,
    /// The cursor it was asked from: a line beginning before it never
    /// matches, even one a backlog replays after the wait was registered.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    since: Option<OutputCursor>,
    /// Whether the start it rides on found the process already running.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    already_running: bool,
}

impl Waiter {
    /// Takes its reply, to answer it.
    pub fn into_reply(self) -> WaitReply {
        self.reply
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::service::supervisor::SpawnGeneration;

    #[test]
    fn a_fresh_pin_skips_the_run_it_replaced() {
        let current = SpawnGeneration::new(4);

        assert!(!RunPin::After(current).counts(current));
        assert!(RunPin::After(current).counts(current.next()));
        assert!(RunPin::From(current).counts(current));
        assert!(!RunPin::From(current).counts(SpawnGeneration::new(3)));
    }
}
