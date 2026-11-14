//! One connection to a holder: the handle, its reader and shared writer.

mod detachable_sink;
pub(crate) mod handle;
#[cfg(test)]
pub(crate) mod losing_sink;
mod reader;
mod replay_request;
mod shared_writer;
#[cfg(test)]
pub(crate) mod watch;

pub use handle::*;
