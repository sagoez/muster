use listeners::{Protocol, SocketState};
use sysinfo::{Pid as SysinfoPid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::domain::{
    port::{PortOccupant, PortOccupants},
    value::Pid,
};

/// Between the arguments of a command line read from the process table.
const ARGUMENT_SEPARATOR: &str = " ";

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
    /// The process listening on TCP `port`. A failed listing reads as none,
    /// so a start is never refused for a holder that cannot be shown.
    fn occupant(&mut self, port: u16) -> Option<PortOccupant> {
        let listener = listeners::get_all().ok()?.into_iter().find(|listener| {
            listener.protocol == Protocol::TCP
                && listener.state == SocketState::Listen
                && listener.socket.port() == port
        })?;
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
        Some(PortOccupant::builder().pid(pid).command(command).build())
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, process};

    use super::*;

    /// A port this test process listens on is found held by this process, and
    /// once closed by nobody.
    #[test]
    fn a_listening_port_names_its_process() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut occupants = OsPortOccupants::default();

        let occupant = occupants.occupant(port).expect("this process listens");

        assert_eq!(occupant.pid().into_inner(), process::id());
        assert!(!occupant.command().is_empty());
        drop(listener);
        assert_eq!(occupants.occupant(port), None);
    }
}
