use getset::CopyGetters;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `muster service stop --json` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ServiceStopped {
    /// Whether a service was running and was stopped.
    stopped: bool,
}
