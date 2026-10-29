//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_limit;
mod byte_queue;
#[cfg(test)]
mod channel_sink;
pub mod client;
mod client_message;
mod client_role;
mod config;
mod dirs;
mod error;
mod error_line;
mod filter_state;
mod frame;
mod frame_tag;
mod generation;
mod handle;
mod holder_control;
mod holder_launcher;
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
mod matcher;
mod pattern_error;
mod piece;
mod protocol;
mod reader;
#[cfg(all(test, target_os = "linux"))]
mod reaped;
mod record;
mod reject_reason;
mod resolved;
mod runner;
mod server;
mod shared_writer;
mod span;
mod span_reader;
mod spawn_request_wire;
mod strip;
#[cfg(test)]
pub(crate) mod testing;

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
pub use server::*;
pub use spawn_request_wire::*;
pub use strip::*;
