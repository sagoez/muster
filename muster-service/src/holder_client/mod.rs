//! The service's side of its holders: launching and adopting them, the
//! handle a supervisor drives a holder through, and the reader that streams
//! a holder's output back.

mod connection;
mod launch;
pub(crate) mod pool;

pub use connection::*;
pub use launch::*;
pub use pool::*;
