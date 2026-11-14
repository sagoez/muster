use serde::Serialize;

/// The room of a project's floor a process stands in, picked by its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Room {
    /// Its child runs and is ready, or has no check to pass.
    Running,
    /// Its last run failed, or its start was refused.
    Failed,
    /// It is launching, or waiting to pass its readiness check.
    Starting,
    /// It is stopping, or waiting to start again.
    Restarting,
    /// Nothing of it runs.
    Off,
}
