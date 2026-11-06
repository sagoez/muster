use muster_domain::port::{Notifier, PortOccupants};
use muster_service::ProjectPorts;

/// The ports of a service run in the test process: nothing of the user's
/// desktop or process table.
pub struct QuietPorts;

impl ProjectPorts for QuietPorts {
    /// No desktop to notify.
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        None
    }

    /// No process table to ask who holds a port.
    fn port_occupants(&self) -> Option<Box<dyn PortOccupants>> {
        None
    }
}
