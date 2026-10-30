use crate::domain::{config::ConfigError, occupancy::PendingIdleStop, value::ProjectRoot};

/// Driven port: the idle stops pending when a service last ran, so the next
/// service stops each project on time rather than forgetting it or starting
/// its wait afresh. Removing a project removes its record.
pub trait PendingIdleStops: Send + Sync {
    /// Every registered project with an idle stop pending; a record whose
    /// folder the domain refuses is skipped.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn pending_idle_stops(&self) -> Result<Vec<PendingIdleStop>, ConfigError>;

    /// Records `stop`, replacing the project's earlier one. A project no
    /// longer registered is skipped.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn arm_idle_stop(&self, stop: &PendingIdleStop) -> Result<(), ConfigError>;

    /// Forgets the pending idle stop of the project at `root`, as a shell
    /// entered it, a client brought it up or the stop ran.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn clear_idle_stop(&self, root: &ProjectRoot) -> Result<(), ConfigError>;
}
