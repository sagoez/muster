use std::collections::HashMap;

use super::ListeningSocket;
use crate::domain::value::Pid;

/// Finds the TCP sockets process trees listen on. A driven port so the
/// domain never names the OS interface that lists them; `&mut self` lets an
/// implementation keep its process table between samples.
pub trait ListeningPorts {
    /// The listening sockets of each root pid's process tree, keyed by that
    /// root and sorted. A root whose tree listens on nothing is absent.
    fn listening(&mut self, roots: &[Pid]) -> HashMap<Pid, Vec<ListeningSocket>>;
}
