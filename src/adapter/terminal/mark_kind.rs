use strum::{Display, EnumString};

/// What a [`StreamMark`](super::stream_mark::StreamMark) records about the
/// point of a process's output it sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum MarkKind {
    /// The alternate screen came up just before this point.
    EnterAlternate,
    /// The screen went back to the primary one just before this point.
    LeaveAlternate,
    /// A run begins here, on a fresh terminal. It sorts after the screen
    /// marks at the same point, which the run before it left.
    RunStart,
}
