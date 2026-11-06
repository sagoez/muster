//! The service's side of its holders: launching and adopting them, the
//! handle a supervisor drives a holder through, and the reader that streams
//! a holder's output back.

mod detachable_sink;
mod handle;
#[cfg(test)]
mod losing_sink;
mod reader;
mod replay_request;
mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
mod shared_writer;
#[cfg(test)]
mod spec_launcher;
#[cfg(test)]
mod watch;

pub use handle::*;
pub use runner::*;
