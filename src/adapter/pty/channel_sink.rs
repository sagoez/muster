use crossbeam_channel::Sender;

use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// Forwards a spawned process's output into a channel.
pub(super) struct ChannelSink(pub(super) Sender<ProcessOutput>);

impl OutputSink for ChannelSink {
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(output);
    }
}
