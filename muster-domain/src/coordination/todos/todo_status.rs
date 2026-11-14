use strum::{Display, EnumString};

/// Where a todo sits in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Actively being worked.
    InProgress,
    /// Finished.
    Done,
}
