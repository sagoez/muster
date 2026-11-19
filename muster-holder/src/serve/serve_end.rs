use muster_protocol::holder::LeftExit;

/// How the holder loop ended.
pub(super) enum ServeEnd {
    /// A supervisor collected the exit, or none ever came by the deadline:
    /// the record goes with the holder.
    Done,
    /// The child exited and no supervisor could reach the holder to collect
    /// it, its socket gone for good: the exit is left beside the record for
    /// the next service to report.
    Stranded(LeftExit),
}
