//! Launching and adopting holders, and the handle of a holder that left its
//! exit because no supervisor could reach it.

mod holder_census;
pub(crate) mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
#[cfg(test)]
mod spec_launcher;
mod stranded_handle;

pub use holder_census::*;
pub use runner::*;
