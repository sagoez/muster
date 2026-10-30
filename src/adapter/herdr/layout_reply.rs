use serde::Deserialize;

/// herdr's answer to `pane layout`:
/// `{"result": {"layout": {"panes": [{"pane_id", "rect": {"width",
/// "height"}}]}}}`, sizes in cells. Only the fields muster reads; herdr may
/// add others.
#[derive(Debug, Deserialize)]
pub(super) struct LayoutReply {
    /// The answer.
    result: LayoutResult,
}

/// The `result` of a [`LayoutReply`].
#[derive(Debug, Deserialize)]
struct LayoutResult {
    /// The layout of the tab the asked-for pane is in.
    layout: Layout,
}

/// The panes of one tab.
#[derive(Debug, Deserialize)]
struct Layout {
    /// Every pane of the tab.
    panes: Vec<LayoutPane>,
}

/// One pane of a [`Layout`].
#[derive(Debug, Deserialize)]
struct LayoutPane {
    /// herdr's id for it.
    pane_id: String,
    /// Where it sits.
    rect: PaneRect,
}

/// A pane's size, in cells.
#[derive(Debug, Deserialize)]
struct PaneRect {
    /// Columns.
    width: u16,
    /// Rows.
    height: u16,
}

impl LayoutReply {
    /// The width and height of `pane`, in cells, when the layout lists it.
    pub(super) fn size_of(&self, pane: &str) -> Option<(u16, u16)> {
        self.result
            .layout
            .panes
            .iter()
            .find(|listed| listed.pane_id == pane)
            .map(|listed| (listed.rect.width, listed.rect.height))
    }
}
