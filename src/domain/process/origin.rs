/// Whether a process came from workspace configuration or a runtime agent
/// session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProcessOrigin {
    /// A persistent process declared in `muster.yml`.
    #[default]
    Configured,
    /// A durable runtime agent session managed outside `muster.yml`.
    Session,
}
