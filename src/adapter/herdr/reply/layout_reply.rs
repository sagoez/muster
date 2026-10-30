use serde::Deserialize;

use super::layout_result::LayoutResult;

/// herdr's answer to `pane layout`:
/// `{"result": {"layout": {"panes": [{"pane_id", "rect": {"width",
/// "height"}}]}}}`, sizes in cells. Only the fields muster reads; herdr may
/// add others.
#[derive(Debug, Deserialize)]
pub struct LayoutReply {
    /// The answer.
    result: LayoutResult,
}

impl LayoutReply {
    /// The width and height of `pane`, in cells, when the layout lists it.
    pub fn size_of(&self, pane: &str) -> Option<(u16, u16)> {
        self.result
            .layout()
            .panes()
            .iter()
            .find(|listed| listed.pane_id() == pane)
            .map(|listed| (listed.rect().width(), listed.rect().height()))
    }
}
