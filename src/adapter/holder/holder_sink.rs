use crossbeam_channel::Sender;
use muster_domain::{port::OutputSink, pty::ProcessOutput};

use super::holder_event::HolderEvent;

/// Forwards the child's output into the holder loop.
pub(super) struct HolderSink(Sender<HolderEvent>);

impl HolderSink {
    /// A sink queueing onto `loop_events`.
    pub(super) fn new(loop_events: Sender<HolderEvent>) -> Self {
        Self(loop_events)
    }
}

impl OutputSink for HolderSink {
    /// Queues one child event; a finished holder simply drops it.
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(HolderEvent::Child(output));
    }
}
