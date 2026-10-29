use std::time::Instant;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::{
    adapter::{holder::LineMatcher, service::ProbeTarget},
    domain::process::{Readiness, ReadyCheck, ReadyProbe},
};

/// The readiness of one pane's current run: its check, how far it got, and
/// what is tested to pass it. Private to the supervisor, which owns it.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub(crate) struct RunReadiness {
    /// How far the run got.
    #[getset(get_copy = "pub(super)")]
    state: Readiness,
    /// The check it is held to.
    #[getset(get = "pub(super)")]
    check: ReadyCheck,
    /// When the run started, for how long it took.
    #[getset(get_copy = "pub(super)")]
    since: Instant,
    /// When it times out while still waiting.
    #[getset(get_copy = "pub(super)")]
    deadline: Instant,
    /// The compiled pattern of a log check.
    #[getset(get = "pub(super)")]
    matcher: Option<LineMatcher>,
}

impl RunReadiness {
    /// The probe a port or HTTP check is tested with.
    pub(super) fn probe(&self) -> Option<ProbeTarget> {
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
    pub(super) fn settle(&mut self, state: Readiness) {
        self.state = state;
        self.matcher = None;
    }
}
