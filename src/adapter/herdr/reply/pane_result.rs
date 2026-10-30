use serde::Deserialize;

use super::pane_info::PaneInfo;

/// The `result` of a [`PaneReply`](super::PaneReply).
#[derive(Debug, Deserialize)]
pub struct PaneResult {
    /// The pane it names.
    pane: PaneInfo,
}

impl PaneResult {
    /// The id of the pane it names.
    pub fn into_pane_id(self) -> String {
        self.pane.into_pane_id()
    }
}
