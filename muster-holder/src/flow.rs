/// Whether the holder loop keeps serving after a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    /// Keep draining events.
    Continue,
    /// A supervisor collected the exit; stop serving.
    Finish,
}
