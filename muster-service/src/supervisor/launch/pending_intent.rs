/// A lifecycle request made while a pane was starting, applied once its
/// launch or adoption settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingIntent {
    /// Stop it through its stop policy.
    Stop,
    /// Kill it at once.
    Kill,
    /// Restart it.
    Restart,
}
