use std::time::SystemTime;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::ProjectRoot;

/// A project whose idle stop is pending: its last shell left at `left_at`,
/// and it stops once its idle stop has passed since.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct PendingIdleStop {
    /// The project.
    project: ProjectRoot,
    /// When its last shell left.
    left_at: SystemTime,
}
