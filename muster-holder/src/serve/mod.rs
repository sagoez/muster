//! The holder loop and what it owns.

pub(crate) mod byte_queue;
mod flow;
pub(crate) mod holder_event;
mod record_guard;
mod reset_warning;
pub(crate) mod server;
#[cfg(any(test, feature = "testing"))]
pub(crate) mod server_testing;
mod socket_guard;

pub use server::*;
