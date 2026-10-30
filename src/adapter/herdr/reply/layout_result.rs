use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::layout::Layout;

/// The `result` of a [`LayoutReply`](super::LayoutReply).
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LayoutResult {
    /// The layout of the tab the asked-for pane is in.
    layout: Layout,
}
