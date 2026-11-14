use std::io::Write;

/// A response written over time on its connection's own thread: its head,
/// then its body as it comes, until its source or its client goes.
pub trait Streamed: Send {
    /// Writes the whole response to `out`, returning once it ends or a
    /// write fails.
    fn stream(self: Box<Self>, out: &mut dyn Write);
}
