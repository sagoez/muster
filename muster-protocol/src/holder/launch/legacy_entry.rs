/// What the sweep of the folder an earlier muster bound holder sockets in
/// does with one entry in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyEntry {
    /// The socket of a holder still running, kept until it exits.
    Running,
    /// A launch's spec or error file, removed at the next sweep: a spec is
    /// read once at launch, and an error file only by the launch waiting on
    /// it, so a holder already serving never needs either.
    Handover,
    /// The socket of a holder that ended, removed at the next sweep.
    Ended,
    /// Anything else, which no sweep removes.
    Stray,
}
