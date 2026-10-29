use serde::{Deserialize, Serialize};

use super::WireForm;
use crate::domain::{port::PortOccupant, value::Pid};

/// A process listening on a port, as the service names a blocked start's
/// holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PortOccupantWire {
    /// Its process id.
    pid: Pid,
    /// Its command line, or its executable.
    command: String,
}

impl WireForm for PortOccupant {
    type Wire = PortOccupantWire;

    fn to_wire(&self) -> PortOccupantWire {
        PortOccupantWire {
            pid: self.pid(),
            command: self.command().clone(),
        }
    }

    fn from_wire(wire: PortOccupantWire) -> Result<Self, String> {
        Ok(Self::builder().pid(wire.pid).command(wire.command).build())
    }
}
