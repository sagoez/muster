use super::{project_reading::ProjectReading, state_trouble::StateTrouble};

/// What reading the registered projects from muster's state found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectsReading {
    /// Every registered project, in the store's order.
    Read(Vec<ProjectReading>),
    /// The state could not be read, and how it stands: no database yet,
    /// one an older or a newer muster wrote, or one that could not be read
    /// this time.
    Trouble(StateTrouble),
}
