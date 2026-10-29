use listeners::{Listener, Protocol, SocketState};
use sysinfo::{
    Pid as SysinfoPid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind,
};

use crate::domain::{
    port::{PortOccupant, PortOccupants},
    value::{LocalPort, Pid},
};

/// Between the arguments of a command line read from the process table.
const ARGUMENT_SEPARATOR: &str = " ";
/// Most ancestors of an occupant read, past any process tree's depth.
const MAX_ANCESTORS: usize = 64;

/// A [`PortOccupants`] over the OS socket tables through `listeners`, naming
/// each occupant by its command line read through `sysinfo`.
pub struct OsPortOccupants {
    /// The process table, refreshed only for the occupant found.
    system: System,
}

impl Default for OsPortOccupants {
    /// A lookup with an empty process table.
    fn default() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl PortOccupants for OsPortOccupants {
    /// The lowest of `ports` a process listens on over TCP, from one listing
    /// of the socket table. A failed listing reads as none, so a start is
    /// never refused for a holder that cannot be shown.
    fn first_occupied(&mut self, ports: &[LocalPort]) -> Option<(LocalPort, PortOccupant)> {
        if ports.is_empty() {
            return None;
        }
        let (port, listener) = listeners::get_all()
            .ok()?
            .into_iter()
            .filter(|listener| {
                listener.protocol == Protocol::TCP && listener.state == SocketState::Listen
            })
            .filter_map(|listener| {
                let port = ports
                    .iter()
                    .copied()
                    .find(|port| port.into_inner() == listener.socket.port())?;
                Some((port, listener))
            })
            .min_by_key(|(port, _)| *port)?;
        Some((port, self.occupant(listener)?))
    }
}

impl OsPortOccupants {
    /// The process behind `listener`, named by its command line and ancestors.
    fn occupant(&mut self, listener: Listener) -> Option<PortOccupant> {
        let pid = Pid::try_new(listener.process.pid).ok()?;
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[SysinfoPid::from_u32(pid.into_inner())]),
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .without_tasks(),
        );
        let command = self
            .system
            .process(SysinfoPid::from_u32(pid.into_inner()))
            .map(|process| {
                process
                    .cmd()
                    .iter()
                    .map(|argument| argument.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(ARGUMENT_SEPARATOR)
            })
            .filter(|command| !command.is_empty())
            .unwrap_or(listener.process.name);
        Some(
            PortOccupant::builder()
                .pid(pid)
                .command(command)
                .ancestors(self.ancestors(pid))
                .build(),
        )
    }

    /// The parent of `pid`, its parent and so on, at most [`MAX_ANCESTORS`]
    /// of them, reading each from the process table as it is reached.
    fn ancestors(&mut self, pid: Pid) -> Vec<Pid> {
        let mut ancestors = Vec::new();
        let mut current = SysinfoPid::from_u32(pid.into_inner());
        while ancestors.len() < MAX_ANCESTORS {
            let Some(parent) = self.system.process(current).and_then(Process::parent) else {
                break;
            };
            let Ok(parent_pid) = Pid::try_new(parent.as_u32()) else {
                break;
            };
            ancestors.push(parent_pid);
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[parent]),
                true,
                ProcessRefreshKind::nothing().without_tasks(),
            );
            current = parent;
        }
        ancestors
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, os::unix::process::parent_id, process};

    use super::*;

    /// Any free port on the loopback, to listen on.
    const ANY_LOOPBACK_PORT: &str = "127.0.0.1:0";

    /// A listener on a free loopback port, with that port.
    fn listening() -> (TcpListener, LocalPort) {
        let listener = TcpListener::bind(ANY_LOOPBACK_PORT).unwrap();
        let port = LocalPort::try_new(listener.local_addr().unwrap().port()).unwrap();
        (listener, port)
    }

    /// A port this test process listens on is found held by this process, and
    /// once closed by nobody.
    #[test]
    fn a_listening_port_names_its_process() {
        let (listener, port) = listening();
        let mut occupants = OsPortOccupants::default();

        let (found, occupant) = occupants
            .first_occupied(&[port])
            .expect("this process listens");

        assert_eq!(found, port);
        assert_eq!(occupant.pid().into_inner(), process::id());
        assert!(!occupant.command().is_empty());
        assert_eq!(
            occupant
                .ancestors()
                .first()
                .map(|parent| parent.into_inner()),
            Some(parent_id()),
            "its parent leads its ancestors"
        );
        drop(listener);
        assert_eq!(occupants.first_occupied(&[port]), None);
    }

    /// Of several ports, the lowest one held is named, a free one among them
    /// is passed over, and no ports asked finds nothing.
    #[test]
    fn os_port_occupants_finds_the_first_held_of_several() {
        let (first, one) = listening();
        let (second, other) = listening();
        let (closed, free) = listening();
        drop(closed);
        let mut ports = vec![one, other, free];
        ports.sort();
        let lowest_held = one.min(other);
        let mut occupants = OsPortOccupants::default();

        let (found, occupant) = occupants
            .first_occupied(&ports)
            .expect("two of the ports are held");

        assert_eq!(found, lowest_held);
        assert_eq!(occupant.pid().into_inner(), process::id());
        assert_eq!(occupants.first_occupied(&[free]), None);
        assert_eq!(occupants.first_occupied(&[]), None);
        drop((first, second));
    }
}
