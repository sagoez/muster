//! The holder protocol and the process log format: what a holder and its
//! clients send over the holder socket, where a holder keeps its socket,
//! record and log, and how a log is written, rotated, filtered and read.

mod error;
mod launch;
mod log;
mod socket;

pub use error::*;
pub use launch::*;
pub use log::*;
pub use socket::*;
