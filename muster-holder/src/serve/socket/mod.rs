//! Keeping the holder's socket: the thread accepting on it, and the looks
//! that bind it again when something removed it.

mod acceptor;
mod acceptor_counts;
mod socket_check;
mod socket_keeper;

pub(super) use socket_check::*;
pub(super) use socket_keeper::*;
