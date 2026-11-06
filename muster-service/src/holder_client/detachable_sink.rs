use std::sync::{Arc, Mutex, PoisonError};

use muster_domain::{port::OutputSink, pty::ProcessOutput};

/// The output sink a holder connection's reader and writer share until its
/// handle is dropped. Detaching makes every send that starts afterwards a
/// no-op, so a reader still probing the holder or reading its log reports
/// nothing, and gives up this hold on the sink at once rather than when the
/// last thread holding it ends. A send already under way at the detach is
/// not stopped: it keeps the sink until it returns, and its output is still
/// delivered, as it may be blocked on a full sink that the loop calling the
/// detach is the one to drain.
pub(super) struct DetachableSink {
    /// The sink, until detached.
    sink: Mutex<Option<Arc<dyn OutputSink>>>,
}

impl DetachableSink {
    /// A sink forwarding to `sink` until detached.
    pub(super) fn new(sink: Box<dyn OutputSink>) -> Self {
        Self {
            sink: Mutex::new(Some(Arc::from(sink))),
        }
    }

    /// Forwards `output` unless detached. The lock is released before the
    /// send, so a send blocked on a full sink never holds up [`Self::detach`],
    /// which the loop draining that sink may be the one calling.
    pub(super) fn send(&self, output: ProcessOutput) {
        let sink = self
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(sink) = sink {
            sink.send(output);
        }
    }

    /// Gives up this hold on the sink, so no send that starts afterwards
    /// forwards anything.
    pub(super) fn detach(&self) {
        let sink = self
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(sink);
    }

    /// Whether the handle was dropped.
    pub(super) fn is_detached(&self) -> bool {
        self.sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{TryRecvError, unbounded};
    use muster_holder::channel_sink::ChannelSink;

    use super::*;

    /// A warning text the tests forward.
    const NOTE: &str = "note";

    /// Detaching drops the sink while another thread still shares the
    /// detachable one, and nothing sent afterwards is forwarded.
    #[test]
    fn detaching_drops_the_sink_while_it_is_still_shared() {
        let (sender, outputs) = unbounded();
        let shared = Arc::new(DetachableSink::new(Box::new(ChannelSink::new(sender))));
        let reader = Arc::clone(&shared);

        reader.send(ProcessOutput::Warning(NOTE.to_string()));
        assert!(!reader.is_detached());
        shared.detach();
        reader.send(ProcessOutput::Warning(NOTE.to_string()));

        assert!(reader.is_detached());
        assert!(matches!(outputs.try_recv(), Ok(ProcessOutput::Warning(text)) if text == NOTE));
        assert_eq!(outputs.try_recv(), Err(TryRecvError::Disconnected));
    }
}
