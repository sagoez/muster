use serde::Deserialize;

use super::pane_result::PaneResult;

/// herdr's answer naming one pane, as `pane split` and `pane get` print it:
/// `{"result": {"pane": {"pane_id"}}}`. Only the fields muster reads;
/// herdr may add others.
#[derive(Debug, Deserialize)]
pub struct PaneReply {
    /// The answer.
    result: PaneResult,
}

impl PaneReply {
    /// The id of the pane the answer names, as herdr wrote it.
    pub fn into_pane_id(self) -> String {
        self.result.into_pane_id()
    }
}
