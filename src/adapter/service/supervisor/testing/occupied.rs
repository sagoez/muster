use crate::domain::port::{PortOccupant, PortOccupants};

/// Answers every port lookup with the same occupant.
pub(crate) struct Occupied(Option<PortOccupant>);

impl Occupied {
    /// Every port held by `occupant`, or by nobody.
    pub(crate) fn by(occupant: Option<PortOccupant>) -> Self {
        Self(occupant)
    }
}

impl PortOccupants for Occupied {
    fn occupant(&mut self, _port: u16) -> Option<PortOccupant> {
        self.0.clone()
    }
}
