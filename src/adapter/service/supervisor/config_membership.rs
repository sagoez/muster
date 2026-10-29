/// Whether a pane still belongs to the current config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfigMembership {
    /// The pane is represented by a current process specification.
    #[default]
    Tracked,
    /// The live pane was removed from config and must disappear when it exits.
    RetireOnExit,
}
