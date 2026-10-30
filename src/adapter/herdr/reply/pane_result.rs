use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::pane_info::PaneInfo;

/// The `result` of a [`PaneReply`](super::PaneReply).
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct PaneResult {
    /// The pane it names.
    pane: PaneInfo,
}
