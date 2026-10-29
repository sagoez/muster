use getset::{Getters, WithSetters};
use typed_builder::TypedBuilder;

use super::stop_signal::StopSignal;
use crate::domain::value::StopGrace;

/// Graceful shutdown policy for one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct StopPolicy {
    /// Graceful signal sent to the process group.
    #[getset(get = "pub", set_with = "pub")]
    signal: StopSignal,
    /// Time allowed for graceful exit before an unconditional force-kill.
    #[getset(get = "pub", set_with = "pub")]
    grace_period: StopGrace,
}
