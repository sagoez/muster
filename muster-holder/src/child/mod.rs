//! The held child: its guard, its output sink, and test doubles of it.

pub(crate) mod child_guard;
#[cfg(test)]
pub(crate) mod fake_child;
pub(crate) mod holder_sink;
#[cfg(test)]
mod recording_input;
