//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_queue;
#[cfg(any(test, feature = "testing"))]
pub mod channel_sink;
mod child_guard;
mod client_id;
mod connected_client;
#[cfg(test)]
mod fake_child;
mod flow;
#[cfg(any(test, feature = "testing"))]
pub mod harness;
mod holder_event;
mod holder_sink;
#[cfg(any(test, feature = "testing"))]
pub mod holders;
mod launcher;
pub mod pty;
mod record_guard;
#[cfg(test)]
mod recording_input;
mod reset_warning;
mod server;
#[cfg(any(test, feature = "testing"))]
mod server_testing;
mod socket_guard;
#[cfg(any(test, feature = "testing"))]
pub mod test_client;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(any(test, feature = "testing"))]
pub mod thread_launcher;

pub use launcher::*;
pub use server::*;
