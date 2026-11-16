//! One connection to a holder: the handle, its reader and shared writer, and
//! the handle of a holder that left its exit because it could not be reached.

mod detachable_sink;
pub(crate) mod handle;
#[cfg(test)]
pub(crate) mod losing_sink;
mod reader;
mod replay_request;
mod shared_writer;
pub(crate) mod stranded_handle;
#[cfg(test)]
pub(crate) mod watch;

pub use handle::*;
