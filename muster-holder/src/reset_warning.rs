/// Where a holder's warning that its child's screen was reset stands: a
/// run is warned once, and only a supervisor taking the warning counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResetWarning {
    /// The screen was never reset.
    #[default]
    Unneeded,
    /// The screen was reset, and no supervisor has been sent the warning.
    Pending,
    /// A supervisor was sent the warning; the run is not warned again.
    Sent,
}
