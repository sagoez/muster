use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::value::StopGrace;

/// Graceful signal sent before a command is force-killed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum StopSignal {
    /// Requests the conventional service shutdown path (SIGTERM on Unix).
    Terminate,
    /// Requests the conventional interactive interrupt path (SIGINT on Unix).
    Interrupt,
}

/// Graceful shutdown policy for one process.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Getters,
    WithSetters,
    TypedBuilder,
)]
#[set_with]
pub struct StopPolicy {
    /// Graceful signal sent to the process group.
    #[getset(get = "pub", set_with = "pub")]
    signal: StopSignal,
    /// Time allowed for graceful exit before an unconditional force-kill.
    #[getset(get = "pub", set_with = "pub")]
    grace_period: StopGrace,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_signals_by_kebab_case_name() {
        assert_eq!(
            "interrupt".parse::<StopSignal>().unwrap(),
            StopSignal::Interrupt
        );
        assert!("kill".parse::<StopSignal>().is_err());
    }
}
