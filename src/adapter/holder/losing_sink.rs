use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// A sink whose receiver is gone: it notes that the exit arrived and delivers
/// nothing.
pub(super) struct LosingSink(Arc<AtomicBool>);

impl LosingSink {
    /// A sink setting `reported` once the exit arrives.
    pub(super) fn new(reported: Arc<AtomicBool>) -> Self {
        Self(reported)
    }
}

impl OutputSink for LosingSink {
    fn send(&self, output: ProcessOutput) {
        if matches!(output, ProcessOutput::Exited(_)) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}
