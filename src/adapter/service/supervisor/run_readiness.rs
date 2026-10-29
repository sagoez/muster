use std::time::Instant;

use crate::{
    adapter::{holder::LineMatcher, service::ProbeTarget},
    domain::process::{Readiness, ReadyCheck, ReadyProbe},
};

/// The readiness of one pane's current run: its check, how far it got, and
/// what is tested to pass it. Private to the supervisor, which owns it.
pub(crate) struct RunReadiness {
    /// How far the run got.
    pub(super) state: Readiness,
    /// The check it is held to.
    pub(super) check: ReadyCheck,
    /// When the run started, for how long it took.
    pub(super) since: Instant,
    /// When it times out while still waiting.
    pub(super) deadline: Instant,
    /// The compiled pattern of a log check.
    pub(super) matcher: Option<LineMatcher>,
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
}
