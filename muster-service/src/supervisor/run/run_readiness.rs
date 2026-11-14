use std::time::Instant;

use getset::{CopyGetters, Getters};
use muster_domain::process::{Readiness, ReadyCheck, ReadyProbe};
use muster_protocol::holder::LineMatcher;
use typed_builder::TypedBuilder;

use crate::ProbeTarget;

/// The readiness of one pane's current run: its check, how far it got, and
/// what is tested to pass it. Private to the supervisor, which owns it.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub(crate) struct RunReadiness {
    /// How far the run got.
    #[getset(get_copy = "pub(crate)")]
    state: Readiness,
    /// The check it is held to.
    #[getset(get = "pub(crate)")]
    check: ReadyCheck,
    /// When the run started, for how long it took.
    #[getset(get_copy = "pub(crate)")]
    since: Instant,
    /// When it times out while still waiting.
    #[getset(get_copy = "pub(crate)")]
    deadline: Instant,
    /// The compiled pattern of a log check.
    #[getset(get = "pub(crate)")]
    matcher: Option<LineMatcher>,
}

impl RunReadiness {
    /// The probe a port or HTTP check is tested with.
    pub(crate) fn probe(&self) -> Option<ProbeTarget> {
        match self.check.probe() {
            ReadyProbe::Log(_) => None,
            ReadyProbe::Port(port) => Some(ProbeTarget::Port(*port)),
            ReadyProbe::Http(url) => Some(ProbeTarget::Http {
                url: url.clone(),
                status: None,
            }),
        }
    }

    /// Moves the check to `state`, which ends it, so its log pattern is no
    /// longer matched.
    pub(crate) fn settle(&mut self, state: Readiness) {
        self.state = state;
        self.matcher = None;
    }
}
