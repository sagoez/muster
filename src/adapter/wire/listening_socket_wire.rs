use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use super::WireForm;
use crate::domain::{port::ListeningSocket, value::Pid};

/// A socket a process tree listens on, as the service and MCP show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ListeningSocketWire {
    /// The port it listens on.
    port: u16,
    /// The address it is bound to.
    address: IpAddr,
    /// The process in the tree that holds the socket.
    pid: Pid,
}

impl WireForm for ListeningSocket {
    type Wire = ListeningSocketWire;

    fn to_wire(&self) -> ListeningSocketWire {
        ListeningSocketWire {
            port: self.port(),
            address: self.address(),
            pid: self.pid(),
        }
    }

    fn from_wire(wire: ListeningSocketWire) -> Result<Self, String> {
        Ok(Self::builder()
            .port(wire.port)
            .address(wire.address)
            .pid(wire.pid)
            .build())
    }
}
