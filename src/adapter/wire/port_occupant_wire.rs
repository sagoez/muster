use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{port::PortOccupant, value::Pid};

/// A process listening on a port, as the service names a blocked start's
/// holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::port::{OCCUPANT_COMMAND_CUT, OCCUPANT_COMMAND_MAX_CHARS};

    /// An occupant read off the wire with a command line past the cap is
    /// cut, so a client never shows more than the cap whatever it is sent.
    #[test]
    fn a_command_line_past_the_cap_is_cut_when_read() {
        let wire = PortOccupantWire::builder()
            .pid(Pid::try_new(4242).unwrap())
            .command("y".repeat(OCCUPANT_COMMAND_MAX_CHARS + 1))
            .build();

        let occupant = PortOccupant::from_wire(wire).unwrap();

        assert_eq!(
            occupant.command(),
            &format!(
                "{}{OCCUPANT_COMMAND_CUT}",
                "y".repeat(OCCUPANT_COMMAND_MAX_CHARS)
            )
        );
    }
}
