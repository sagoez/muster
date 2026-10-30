use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// Discards a detached handle's output.
pub(super) struct NullSink;

impl OutputSink for NullSink {
    fn send(&self, _output: ProcessOutput) {}
}
