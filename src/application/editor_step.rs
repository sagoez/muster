use crate::domain::{coordination::Author, value::ProjectRoot};

/// One step the editor fakes record, so a test can check what happened in
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EditorStep {
    /// A store committed a write.
    Wrote,
    /// The stack store committed a write for this author.
    WroteFor(Author),
    /// The notifier heard a folder registered as a project.
    ProjectRegistered(ProjectRoot),
    /// The notifier heard a stack change.
    StackChanged(ProjectRoot),
    /// The notifier heard a settings change.
    SettingsChanged,
}
