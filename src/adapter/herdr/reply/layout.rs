use getset::Getters;
use serde::Deserialize;

use super::layout_pane::LayoutPane;

/// The panes of one tab.
#[derive(Debug, Deserialize, Getters)]
pub struct Layout {
    /// Every pane of the tab.
    #[getset(get = "pub")]
    panes: Vec<LayoutPane>,
}
