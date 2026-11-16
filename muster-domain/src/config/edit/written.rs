use getset::Getters;
use typed_builder::TypedBuilder;

use crate::project::Project;

/// What a shared write answered, and the project it registered in the same
/// transaction, if it registered one.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Written<T> {
    /// The write's own answer.
    value: T,
    /// The project the write registered for itself.
    registered: Option<Project>,
}

impl<T> Written<T> {
    /// The write's answer and the project it registered, taken apart.
    pub fn into_parts(self) -> (T, Option<Project>) {
        (self.value, self.registered)
    }
}
