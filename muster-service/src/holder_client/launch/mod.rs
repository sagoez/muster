//! Launching and adopting holders.

pub(crate) mod runner;
#[cfg(test)]
mod runner_fixture;
#[cfg(test)]
mod sabotage;
#[cfg(test)]
mod spec_launcher;

pub use runner::*;
