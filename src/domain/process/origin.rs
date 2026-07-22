/// Whether a process came from workspace configuration or the live TUI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProcessOrigin {
    /// A persistent process declared in `muster.yml`.
    #[default]
    Configured,
    /// A disposable process created for the current TUI session.
    Session,
}
