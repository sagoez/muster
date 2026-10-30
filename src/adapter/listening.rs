use std::collections::HashMap;

use listeners::{Protocol, SocketState};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::process_tree::ProcessTree;
use crate::domain::{
    port::{ListeningPorts, ListeningSocket},
    value::Pid,
};

/// A [`ListeningPorts`] over the OS socket tables (`/proc` on Linux,
/// `libproc` on macOS) through `listeners`, attributing each listener to the
/// supervised root whose process tree, walked through `sysinfo`, holds it.
pub struct OsListeningPorts {
    /// The process table, kept between samples so a refresh only updates it.
    system: System,
}

impl Default for OsListeningPorts {
    /// A sampler with an empty process table.
    fn default() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl ListeningPorts for OsListeningPorts {
    /// The listening TCP sockets under each root. A failed listing reads as
    /// none: a missed sample only delays what the next one finds.
    fn listening(&mut self, roots: &[Pid]) -> HashMap<Pid, Vec<ListeningSocket>> {
        if roots.is_empty() {
            return HashMap::new();
        }
        let Ok(listeners) = listeners::get_all() else {
            return HashMap::new();
        };
        let mut tcp = listeners
            .into_iter()
            .filter(|listener| {
                listener.protocol == Protocol::TCP && listener.state == SocketState::Listen
            })
            .peekable();
        if tcp.peek().is_none() {
            return HashMap::new();
        }
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        let tree = ProcessTree::of(&self.system);
        let mut owners = HashMap::new();
        for &root in roots {
            for pid in tree.subtree(root) {
                owners.entry(pid.as_u32()).or_insert(root);
            }
        }
        let mut found: HashMap<Pid, Vec<ListeningSocket>> = HashMap::new();
        for listener in tcp {
            let Ok(pid) = Pid::try_new(listener.process.pid) else {
                continue;
            };
            if let Some(&root) = owners.get(&pid.into_inner()) {
                found.entry(root).or_default().push(
                    ListeningSocket::builder()
                        .port(listener.socket.port())
                        .address(listener.socket.ip())
                        .pid(pid)
                        .build(),
                );
            }
        }
        for sockets in found.values_mut() {
            sockets.sort();
            sockets.dedup();
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, process};

    use super::*;

    /// A socket this test process listens on is found under its own pid, and
    /// under no root whose tree does not hold it.
    #[test]
    fn a_listening_socket_is_attributed_to_its_tree() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let me = Pid::try_new(process::id()).unwrap();
        let mut ports = OsListeningPorts::default();

        let found = ports.listening(&[me]);
        let mine = found.get(&me).expect("this process listens");
        assert!(
            mine.iter()
                .any(|socket| socket.port() == port && socket.pid() == me),
            "{mine:?}"
        );

        assert!(ports.listening(&[]).is_empty());
        drop(listener);
        let after = ports.listening(&[me]);
        assert!(
            !after
                .get(&me)
                .is_some_and(|sockets| sockets.iter().any(|socket| socket.port() == port)),
            "a closed socket is no longer reported"
        );
    }
}
