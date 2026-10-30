use std::sync::atomic::AtomicUsize;

use getset::Getters;
use typed_builder::TypedBuilder;

/// Counts stop attempts for a handle whose first hard kill fails.
#[derive(Default, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RetryStopSignals {
    terminates: AtomicUsize,
    kills: AtomicUsize,
}
