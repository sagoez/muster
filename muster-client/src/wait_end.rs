use serde::Serialize;
use strum::Display;

/// How the wait for the processes an action touched ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum WaitEnd {
    /// Every process settled.
    Settled,
    /// The action's own settle timeout expired first, so a process still
    /// starting or stopping failed to settle in time.
    TimedOut,
    /// The caller's shorter bound expired first, so a process still starting
    /// or stopping is on its way rather than failed.
    CutShort,
}
