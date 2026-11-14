use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

/// A stored project whose row the domain refuses, such as one whose name was
/// edited by hand: listed by the folder it is stored at, so it can be named
/// and removed while every other project keeps working.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct UnreadableProject {
    /// The folder the row is stored at, as stored.
    root: PathBuf,
    /// Why the row is refused.
    reason: String,
}
