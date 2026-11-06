use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::pane_rect::PaneRect;

/// One pane of a [`Layout`](super::layout::Layout).
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LayoutPane {
    /// herdr's id for it.
    pane_id: String,
    /// Where it sits.
    rect: PaneRect,
}
