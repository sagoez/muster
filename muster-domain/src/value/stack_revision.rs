use nutype::nutype;

/// Which write a project's stored stack is at. Every write to the stack moves
/// it, so a reader holding an older one knows its copy is stale.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display))]
pub struct StackRevision(i64);
