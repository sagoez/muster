//! A process's log on disk: its generations, rotation, and the line held in
//! place of an alternate screen.

#[cfg(any(test, feature = "testing"))]
pub mod gated_log;
pub(crate) mod generation;
pub(crate) mod log;
pub(crate) mod log_base;
#[cfg(any(test, feature = "testing"))]
pub mod log_fixture;
mod piece;
mod resolved;
pub(crate) mod screen_gate;
pub(crate) mod span;
pub(crate) mod span_reader;

pub use log::*;
pub use screen_gate::*;
