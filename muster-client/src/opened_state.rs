use std::{fmt, sync::Arc};

use muster_domain::port::LocalState;

/// What opening muster's state read-only found. It mirrors muster-store's
/// `ExistingDatabase`, but holds the state as its port, since no driver names
/// the concrete store: the composition root maps the one onto the other.
pub enum OpenedState {
    /// There is no database yet.
    Absent,
    /// An older muster wrote it, and no command has migrated it since.
    Older,
    /// It is at the current schema, open.
    Current(Arc<dyn LocalState>),
}

impl OpenedState {
    /// The state, when it is at the current schema.
    #[must_use]
    pub fn current(self) -> Option<Arc<dyn LocalState>> {
        match self {
            Self::Current(state) => Some(state),
            Self::Absent | Self::Older => None,
        }
    }
}

impl fmt::Debug for OpenedState {
    /// Writes which it is, the open state shown only by name, since the
    /// state port is not `Debug`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Absent => "Absent",
            Self::Older => "Older",
            Self::Current(_) => "Current",
        };
        formatter.write_str(name)
    }
}
