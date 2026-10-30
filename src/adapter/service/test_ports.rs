use super::project_ports::ProjectPorts;
use crate::domain::port::{Notifier, PortOccupants};

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
