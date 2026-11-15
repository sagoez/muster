use super::stream_out::StreamOut;

/// A response written over time on its connection's own thread: its head,
/// then its body as it comes, until its source or its client goes.
pub trait Streamed: Send {
    /// Writes the whole response to `out`, returning once it ends, a write
    /// fails or the client hangs up.
    fn stream(self: Box<Self>, out: &mut dyn StreamOut);
}
