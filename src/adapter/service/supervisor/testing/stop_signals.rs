use std::{
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

use getset::Getters;
use muster_domain::process::StopSignal;
use typed_builder::TypedBuilder;

/// Counts graceful termination requests separately from hard kills.
#[derive(Default, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct StopSignals {
    terminates: AtomicUsize,
    kills: AtomicUsize,
    termination: Mutex<Option<(StopSignal, Duration)>>,
}
