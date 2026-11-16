//! Launching and adopting holders.

mod holder_census;
pub(crate) mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
#[cfg(test)]
mod spec_launcher;

pub use holder_census::*;
pub use runner::*;
