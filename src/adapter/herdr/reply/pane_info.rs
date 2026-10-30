use serde::Deserialize;

/// The pane a [`PaneReply`](super::PaneReply) names.
#[derive(Debug, Deserialize)]
pub struct PaneInfo {
    /// herdr's id for it.
    pane_id: String,
}

impl PaneInfo {
    /// herdr's id for the pane.
    pub fn into_pane_id(self) -> String {
        self.pane_id
    }
}
