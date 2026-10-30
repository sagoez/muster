use crate::domain::value::ProjectRoot;

/// How a project's occupancy moved: it gained its first shell or lost its
/// last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OccupancyChange {
    /// The project had no shell and now has one.
    Entered(ProjectRoot),
    /// The project's last shell left it.
    Left(ProjectRoot),
}
