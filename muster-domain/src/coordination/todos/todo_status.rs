use strum::{Display, EnumIter, EnumString};

/// Where a todo sits in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Actively being worked.
    InProgress,
    /// Finished.
    Done,
}
