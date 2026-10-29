use crossbeam_channel::Sender;

use super::holder_event::HolderEvent;
use crate::domain::{port::OutputSink, pty::ProcessOutput};

/// Forwards the child's output into the holder loop.
pub(super) struct HolderSink(pub(super) Sender<HolderEvent>);

impl OutputSink for HolderSink {
    /// Queues one child event; a finished holder simply drops it.
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(HolderEvent::Child(output));
    }
}
