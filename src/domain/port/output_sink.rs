use crate::domain::pty::ProcessOutput;

/// Sink for a single process's output events, supplied by the runtime. The
/// concrete implementation (e.g. a channel into the event loop) lives in an
/// adapter, keeping the domain free of any transport type.
pub trait OutputSink: Send + Sync + 'static {
    /// Delivers one output event from the process to the runtime.
    fn send(&self, output: ProcessOutput);
}
