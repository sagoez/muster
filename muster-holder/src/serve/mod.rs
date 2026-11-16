//! The holder loop and what it owns.

mod acceptor;
pub(crate) mod byte_queue;
mod flow;
pub(crate) mod holder_event;
mod record_guard;
mod reset_warning;
mod serve_end;
pub(crate) mod server;
#[cfg(any(test, feature = "testing"))]
pub(crate) mod server_testing;
mod socket_check;
mod socket_keeper;
mod socket_trouble;

pub use server::*;
