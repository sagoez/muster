use crossbeam_channel::Sender;

use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// Forwards a spawned process's output into a channel.
pub(crate) struct ChannelSink(Sender<ProcessOutput>);

impl ChannelSink {
    /// A sink delivering every output to `sender`.
    pub(crate) fn new(sender: Sender<ProcessOutput>) -> Self {
        Self(sender)
    }
}

impl OutputSink for ChannelSink {
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(output);
    }
}
