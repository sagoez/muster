use super::project_reading::ProjectReading;

/// What reading the registered projects from muster's state found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectsReading {
    /// Every registered project, in the store's order.
    Read(Vec<ProjectReading>),
    /// muster has no database yet.
    NoState,
    /// A newer muster wrote the database, which this one does not read.
    NewerSchema,
    /// The database could not be read this time.
    Unreadable,
}
