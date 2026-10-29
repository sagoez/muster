use std::fmt;

use serde::{Deserialize, Serialize};

use super::port_holder::PortHolder;
use crate::domain::port::PortOccupant;

/// Who listens on a port a start needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortOwner {
    /// A process muster supervises, named by its project.
    Supervised(PortHolder),
    /// A process muster does not supervise.
    Other(PortOccupant),
}

impl fmt::Display for PortOwner {
    /// Writes `project 'admin', process 'web'` for a supervised process, and
    /// `pid 4242 (python3 -m http.server 3000)` for any other.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Supervised(holder) => holder.fmt(formatter),
            Self::Other(occupant) => {
                write!(formatter, "pid {} ({})", occupant.pid(), occupant.command())
            },
        }
    }
}
