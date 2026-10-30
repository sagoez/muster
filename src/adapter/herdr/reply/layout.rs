use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::layout_pane::LayoutPane;

/// The panes of one tab.
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Layout {
    /// Every pane of the tab.
    panes: Vec<LayoutPane>,
}
