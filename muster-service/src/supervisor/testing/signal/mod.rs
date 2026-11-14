//! Kills, pauses and stop signals.

pub(crate) mod kill_count_handle;
mod kill_count_runner;
mod pause_fail_handle;
mod pause_fail_runner;
mod signal_adopter;

pub(crate) use kill_count_handle::*;
pub(crate) use kill_count_runner::*;
pub(crate) use pause_fail_runner::*;
pub(crate) use signal_adopter::*;
