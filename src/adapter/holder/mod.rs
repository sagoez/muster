//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_limit;
mod byte_queue;
mod child_guard;
pub mod client;
mod client_id;
mod client_message;
mod client_role;
mod config;
mod connected_client;
mod dirs;
mod error;
mod error_line;
#[cfg(test)]
mod fake_child;
mod filter_state;
mod flow;
mod frame;
mod frame_tag;
mod generation;
mod handle;
#[cfg(test)]
pub(crate) mod harness;
mod holder_control;
mod holder_event;
mod holder_launcher;
mod holder_sink;
#[cfg(test)]
pub(crate) mod holders;
mod id;
mod kept;
mod launcher;
mod line_filter;
mod line_limit;
mod line_matcher;
mod log;
mod log_base;
#[cfg(test)]
mod log_fixture;
mod log_query;
mod log_reader;
mod log_slice;
#[cfg(test)]
mod losing_sink;
mod matcher;
mod pattern_error;
mod piece;
mod protocol;
mod reader;
#[cfg(all(test, target_os = "linux"))]
mod reaped;
mod record;
mod record_guard;
#[cfg(test)]
mod recording_input;
mod reject_reason;
mod resolved;
mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
mod served_screen;
mod server;
#[cfg(test)]
mod server_testing;
mod shared_writer;
mod socket_guard;
mod span;
mod span_reader;
mod spawn_request_wire;
#[cfg(test)]
mod spec_launcher;
mod strip;
#[cfg(test)]
mod test_client;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
pub(crate) mod thread_launcher;
#[cfg(test)]
mod watch;

pub use byte_limit::*;
pub use client_message::*;
pub use client_role::*;
pub use config::*;
pub use dirs::*;
pub use error::*;
pub use frame::*;
pub use frame_tag::*;
pub use handle::*;
pub use holder_control::*;
pub use holder_launcher::*;
pub use id::*;
pub use launcher::*;
pub use line_filter::*;
pub use line_limit::*;
pub use line_matcher::*;
pub use log::*;
pub use log_query::*;
pub use log_reader::*;
pub use log_slice::*;
pub use pattern_error::*;
pub use protocol::*;
pub use record::*;
pub use reject_reason::*;
pub use runner::*;
pub use served_screen::*;
pub use server::*;
pub use spawn_request_wire::*;
pub use strip::*;
