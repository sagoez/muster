use std::sync::atomic::AtomicUsize;

/// Counts stop attempts for a handle whose first hard kill fails.
#[derive(Default)]
pub(crate) struct RetryStopSignals {
    pub(crate) terminates: AtomicUsize,
    pub(crate) kills: AtomicUsize,
}
