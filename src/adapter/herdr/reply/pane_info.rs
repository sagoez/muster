use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The pane a [`PaneReply`](super::PaneReply) names.
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct PaneInfo {
    /// herdr's id for it.
    pane_id: String,
}
