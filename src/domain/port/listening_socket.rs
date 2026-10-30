use std::net::IpAddr;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// One TCP socket a supervised process tree is listening on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ListeningSocket {
    /// The port it listens on.
    port: u16,
    /// The address it is bound to.
    address: IpAddr,
    /// The process in the tree that holds the socket.
    pid: Pid,
}
