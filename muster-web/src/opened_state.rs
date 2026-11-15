use std::sync::Arc;

use muster_domain::port::LocalState;

/// What opening muster's state read-only found.
pub enum OpenedState {
    /// There is no database yet.
    Absent,
    /// An older muster wrote it, and no command has migrated it since.
    Older,
    /// It is at the current schema, open.
    Current(Arc<dyn LocalState>),
}
