use muster_domain::port::{Notifier, PortOccupants};

use super::project_ports::ProjectPorts;

/// Ports with nothing wired.
pub(super) struct TestPorts;

impl ProjectPorts for TestPorts {
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        None
    }

    fn port_occupants(&self) -> Option<Box<dyn PortOccupants>> {
        None
    }
}
