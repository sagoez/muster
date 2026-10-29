use std::fmt;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::port_owner::PortOwner;
use crate::domain::value::LocalPort;

/// Opens the reason a start was refused for a held port, before the port.
const BLOCKED_REASON: &str = "did not start: port";
/// Between the port and who holds it in that reason.
const HELD_BY: &str = "is held by";

/// Why a process was not started: another process, supervised or not,
/// listens on the first port it needs that was found taken, declared or
/// probed for readiness.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct PortBlock {
    /// The port it needs.
    #[getset(get_copy = "pub")]
    port: LocalPort,
    /// Who listens on it.
    #[getset(get = "pub")]
    holder: PortOwner,
}

impl fmt::Display for PortBlock {
    /// Writes the refusal for a person: `did not start: port 3000 is held by
    /// project 'admin', process 'web'`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{BLOCKED_REASON} {} {HELD_BY} {}",
            self.port, self.holder
        )
    }
}
