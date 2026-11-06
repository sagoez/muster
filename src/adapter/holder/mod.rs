//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_queue;
mod child_guard;
mod client_id;
mod connected_client;
mod detachable_sink;
#[cfg(test)]
mod fake_child;
mod flow;
mod handle;
#[cfg(test)]
pub(crate) mod harness;
mod holder_event;
mod holder_sink;
#[cfg(test)]
pub(crate) mod holders;
mod launcher;
#[cfg(test)]
mod losing_sink;
mod reader;
mod record_guard;
#[cfg(test)]
mod recording_input;
mod replay_request;
mod reset_warning;
mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
mod server;
#[cfg(test)]
mod server_testing;
mod shared_writer;
mod socket_guard;
#[cfg(test)]
mod spec_launcher;
#[cfg(test)]
mod test_client;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
pub(crate) mod thread_launcher;
#[cfg(test)]
mod watch;

pub use handle::*;
pub use launcher::*;
pub use runner::*;
pub use server::*;
