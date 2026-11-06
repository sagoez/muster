use getset::CopyGetters;
use muster_domain::process::{StopPolicy, StopSignal};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// How a process is stopped, as `--json` shows it. The grace is in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct StopView {
    /// The signal a graceful stop sends.
    #[serde(with = "muster_protocol::wire::form")]
    signal: StopSignal,
    /// Seconds a graceful stop waits before it kills.
    grace: f64,
}

impl From<&StopPolicy> for StopView {
    fn from(stop: &StopPolicy) -> Self {
        Self {
            signal: *stop.signal(),
            grace: stop.grace_period().into_inner().as_secs_f64(),
        }
    }
}
