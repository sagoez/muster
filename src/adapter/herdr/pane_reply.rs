use serde::Deserialize;

/// herdr's answer naming one pane, as `pane split` and `pane get` print it:
/// `{"result": {"pane": {"pane_id"}}}`. Only the fields muster reads;
/// herdr may add others.
#[derive(Debug, Deserialize)]
pub(super) struct PaneReply {
    /// The answer.
    result: PaneResult,
}

/// The `result` of a [`PaneReply`].
#[derive(Debug, Deserialize)]
struct PaneResult {
    /// The pane it names.
    pane: PaneInfo,
}

/// The pane a [`PaneReply`] names.
#[derive(Debug, Deserialize)]
struct PaneInfo {
    /// herdr's id for it.
    pane_id: String,
}

impl PaneReply {
    /// The id of the pane the answer names, as herdr wrote it.
    pub(super) fn into_pane_id(self) -> String {
        self.result.pane.pane_id
    }
}
