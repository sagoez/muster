use super::{CoordinationStore, ProjectStore, SettingsStore, StackEditLog, StackStore};

/// Driven port: all of muster's local state as one handle, so a driver is
/// given the state rather than the database that keeps it.
pub trait LocalState:
    ProjectStore + StackStore + SettingsStore + CoordinationStore + StackEditLog + Send + Sync
{
}

impl<T> LocalState for T where
    T: ProjectStore
        + StackStore
        + SettingsStore
        + CoordinationStore
        + StackEditLog
        + Send
        + Sync
        + ?Sized
{
}
