use std::path::PathBuf;

/// How a [`SpecLauncher`](super::spec_launcher::SpecLauncher) breaks the
/// holder it starts.
pub(super) enum Sabotage {
    /// An unreadable spec, as a service and holder binary that disagree on
    /// its shape would hand over.
    CorruptSpec,
    /// The child's working directory disappears after the runner checked it,
    /// so the PTY spawn fails.
    RemoveDir(PathBuf),
}
