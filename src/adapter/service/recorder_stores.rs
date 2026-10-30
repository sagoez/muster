use typed_builder::TypedBuilder;

use crate::domain::port::{PendingIdleStops, RaisedEdits, RunHistory};

/// The stores a [`StoreRecorder`](super::store_recorder::StoreRecorder)
/// writes to from its own thread. No `Getters`: the recorder's thread takes
/// it whole.
#[derive(TypedBuilder)]
pub struct RecorderStores {
    /// Takes how runs end.
    pub(super) runs: Box<dyn RunHistory>,
    /// Takes which agent edits were raised.
    pub(super) raised: Box<dyn RaisedEdits>,
    /// Takes which idle stops are pending.
    pub(super) idle_stops: Box<dyn PendingIdleStops>,
}
