use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::settings::Settings;

/// The settings a change left stored, and whether it changed them: a change
/// to the value already stored changes nothing, so it is neither logged nor
/// announced.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct AppliedSettings {
    /// The settings as stored after the change.
    #[getset(get = "pub")]
    settings: Settings,
    /// Whether the stored settings differ from before.
    #[getset(get_copy = "pub")]
    changed: bool,
}
