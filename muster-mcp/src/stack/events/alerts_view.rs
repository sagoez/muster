use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::event_view::EventView;

/// The alerts a stack or process result carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct AlertsView {
    /// Alert-worthy events since the last call, newest last.
    #[getset(get = "pub")]
    alerts: Vec<EventView>,
    /// How many more happened than are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    alerts_omitted: Option<usize>,
    /// Present, and true, when events since the last call were dropped
    /// before they were read, so alerts among them were missed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    alerts_gap: Option<bool>,
}
