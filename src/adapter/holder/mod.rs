//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_limit;
mod byte_queue;
pub mod client;
mod config;
mod dirs;
mod error;
mod error_line;
mod frame;
mod handle;
mod id;
mod launcher;
mod line_filter;
mod line_limit;
mod line_matcher;
mod log;
mod log_base;
mod log_query;
mod log_reader;
mod log_slice;
mod pattern_error;
mod protocol;
mod record;
mod runner;
mod server;
mod strip;
#[cfg(test)]
pub(crate) mod testing;

pub use byte_limit::*;
pub use config::*;
pub use dirs::*;
pub use error::*;
pub use frame::*;
pub use handle::*;
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
pub use runner::*;
pub use server::*;
pub use strip::*;
