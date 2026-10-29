use super::{project_ports::ProjectPorts, supervisor::testing::Occupied};
use crate::domain::port::{Notifier, PortOccupant, PortOccupants};

/// Ports whose every lookup finds the port held by one process, and no
/// notifier.
pub(super) struct OccupiedPorts {
    /// Who holds every port.
    occupant: PortOccupant,
}

impl OccupiedPorts {
    /// Every port held by `occupant`.
    pub(super) fn by(occupant: PortOccupant) -> Self {
        Self { occupant }
    }
}

impl ProjectPorts for OccupiedPorts {
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        None
    }

    fn port_occupants(&self) -> Option<Box<dyn PortOccupants>> {
        Some(Box::new(Occupied::by(Some(self.occupant.clone()))))
    }
}
