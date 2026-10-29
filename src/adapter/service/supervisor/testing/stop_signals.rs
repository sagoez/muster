use std::{
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

use crate::domain::process::StopSignal;

/// Counts graceful termination requests separately from hard kills.
#[derive(Default)]
pub(crate) struct StopSignals {
    pub(crate) terminates: AtomicUsize,
    pub(crate) kills: AtomicUsize,
    pub(crate) termination: Mutex<Option<(StopSignal, Duration)>>,
}
