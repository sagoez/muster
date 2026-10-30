use getset::Getters;
use serde::Deserialize;

use super::layout::Layout;

/// The `result` of a [`LayoutReply`](super::LayoutReply).
#[derive(Debug, Deserialize, Getters)]
pub struct LayoutResult {
    /// The layout of the tab the asked-for pane is in.
    #[getset(get = "pub")]
    layout: Layout,
}
