use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{coordination::Author, value::ProjectRoot};

/// A folder no project covered when a shared write there was resolved, and
/// the repository root at or above it that the write registers.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Registering {
    /// The folder the write came from.
    folder: ProjectRoot,
    /// The repository root at or above `folder`, registered unless a project
    /// registered since contains `folder`.
    root: ProjectRoot,
    /// Who writes, named in the edit log when an agent registers.
    by: Author,
}
