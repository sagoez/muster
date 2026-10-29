use super::PortOccupant;
use crate::domain::value::LocalPort;

/// Finds who listens on a port, whoever started it. A driven port so the
/// domain never names the OS interface that lists sockets; `&mut self` lets
/// an implementation keep a process table between lookups. A lookup scans
/// the whole socket table, so it runs on a launch worker, never on a loop.
pub trait PortOccupants: Send {
    /// The first of `ports` (ascending) a process listens on over TCP, with
    /// that process; one socket-table scan however many ports are asked.
    fn first_occupied(&mut self, ports: &[LocalPort]) -> Option<(LocalPort, PortOccupant)>;
}
