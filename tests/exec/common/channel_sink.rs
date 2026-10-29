use crossbeam_channel::Sender;
use muster::domain::{port::OutputSink, pty::ProcessOutput};

/// Forwards a handle's output into a channel.
pub struct ChannelSink(Sender<ProcessOutput>);

impl ChannelSink {
    /// A sink sending each output on `outputs`.
    pub fn new(outputs: Sender<ProcessOutput>) -> Self {
        Self(outputs)
    }
}

impl OutputSink for ChannelSink {
    /// Sends `output` on; a test that stopped listening simply drops it.
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(output);
    }
}
