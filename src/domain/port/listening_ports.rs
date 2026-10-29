use std::{collections::HashMap, net::IpAddr};

use getset::CopyGetters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// One TCP socket a supervised process tree is listening on.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    CopyGetters,
    TypedBuilder,
)]
#[getset(get_copy = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ListeningSocket {
    /// The port it listens on.
    port: u16,
    /// The address it is bound to.
    address: IpAddr,
    /// The process in the tree that holds the socket.
    pid: Pid,
}

/// Finds the TCP sockets process trees listen on. A driven port so the
/// domain never names the OS interface that lists them; `&mut self` lets an
/// implementation keep its process table between samples.
pub trait ListeningPorts {
    /// The listening sockets of each root pid's process tree, keyed by that
    /// root and sorted. A root whose tree listens on nothing is absent.
    fn listening(&mut self, roots: &[Pid]) -> HashMap<Pid, Vec<ListeningSocket>>;
}
