use std::{
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::process::StopSignal;

/// Counts graceful termination requests separately from hard kills.
#[derive(Default, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct StopSignals {
    terminates: AtomicUsize,
    kills: AtomicUsize,
    termination: Mutex<Option<(StopSignal, Duration)>>,
}
