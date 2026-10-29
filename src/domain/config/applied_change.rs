use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::config::ProcessSpec;

/// The definition a change left stored, and whether it changed it: a change
/// to the values already stored changes nothing, so it succeeds saying so and
/// is neither logged nor announced.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct AppliedChange {
    /// The definition as stored after the change.
    #[getset(get = "pub")]
    spec: ProcessSpec,
    /// Whether the stored definition differs from before.
    #[getset(get_copy = "pub")]
    changed: bool,
}
