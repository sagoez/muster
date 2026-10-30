/// Whether a pane still belongs to the project's stack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfigMembership {
    /// The pane is represented by a process in the stack.
    #[default]
    Tracked,
    /// The live pane was removed from the stack and must disappear when it
    /// exits.
    RetireOnExit,
}
