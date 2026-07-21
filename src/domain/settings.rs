use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// User settings that apply across every workspace, stored per machine rather
/// than in any single project's config.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct Settings {
    /// Whether to raise OS desktop notifications. In-app status-bar notices are
    /// always shown regardless.
    #[getset(get = "pub", set_with = "pub")]
    desktop_notifications: bool,
}
