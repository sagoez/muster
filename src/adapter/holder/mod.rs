//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod byte_queue;
pub mod client;
mod config;
mod dirs;
mod error;
mod frame;
mod handle;
mod id;
mod launcher;
mod log;
mod protocol;
mod record;
mod runner;
mod server;
#[cfg(test)]
pub(crate) mod testing;

pub use config::*;
pub use dirs::*;
pub use error::*;
pub use frame::*;
pub use handle::*;
pub use id::*;
pub use launcher::*;
pub use log::*;
pub use protocol::*;
pub use record::*;
pub use runner::*;
pub use server::*;
