use crate::domain::port::{Notifier, PortOccupants};

/// Builds the driven adapters each project's supervisor runs on, as the
/// service loads that project. An adapter that is absent is not wired.
pub trait ProjectPorts {
    /// Desktop notifications.
    fn notifier(&self) -> Option<Box<dyn Notifier>>;

    /// Who listens on a port a start needs, whoever started it.
    fn port_occupants(&self) -> Option<Box<dyn PortOccupants>>;
}
