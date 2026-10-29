use super::PortOccupant;

/// Finds who listens on a port, whoever started it. A driven port so the
/// domain never names the OS interface that lists sockets; `&mut self` lets
/// an implementation keep a process table between lookups. A lookup scans
/// the whole socket table, so it runs on a launch worker, never on a loop.
pub trait PortOccupants: Send {
    /// A process listening on TCP `port`, if any.
    fn occupant(&mut self, port: u16) -> Option<PortOccupant>;
}
