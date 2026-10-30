use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
};

use crate::domain::{
    port::{ListeningPorts, ListeningSocket},
    value::Pid,
};

/// The port every root is reported listening on.
pub(super) const HELD_PORT: u16 = 3000;

/// Reports [`HELD_PORT`] under every root it is asked about, held by the
/// root's first child.
pub(super) struct FixedListeningPorts;

impl ListeningPorts for FixedListeningPorts {
    fn listening(&mut self, roots: &[Pid]) -> HashMap<Pid, Vec<ListeningSocket>> {
        roots
            .iter()
            .map(|root| {
                (*root, vec![
                    ListeningSocket::builder()
                        .port(HELD_PORT)
                        .address(IpAddr::V4(Ipv4Addr::LOCALHOST))
                        .pid(Pid::try_new(root.into_inner() + 1).unwrap())
                        .build(),
                ])
            })
            .collect()
    }
}
