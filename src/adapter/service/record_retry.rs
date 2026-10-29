use std::time::Duration;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Pause before the first retry of a record the store was too busy to take.
pub const RECORD_FIRST_RETRY: Duration = Duration::from_millis(100);
/// Longest pause between retries of a record the store stays too busy for.
pub const RECORD_MAX_RETRY: Duration = Duration::from_secs(5);
/// Longest a record is retried while the store stays busy before it is given
/// up, so a lock never held back the records after it for longer.
pub const RECORD_RETRY_BUDGET: Duration = Duration::from_secs(30);
/// Longest a flush on stop lets the records before it keep trying: past it,
/// each record still waiting is given up and reported.
pub const RECORD_DRAIN_BOUND: Duration = Duration::from_secs(5);
/// Longest a flush waits past [`RECORD_DRAIN_BOUND`] for a write already in
/// the store's hands: past the store's own wait for another writer's lock.
pub const RECORD_FLUSH_SETTLE: Duration = Duration::from_secs(10);

/// How a [`StoreRecorder`](super::StoreRecorder) retries a record the store
/// was too busy to take: the pause doubles from `first` up to `max`, and once
/// the next pause would carry the record past `budget` it is given up so
/// later ones can land. A flush lets records keep trying for `drain` only,
/// and waits at most `settle` past it for the write in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct RecordRetry {
    /// Pause before the first retry.
    first: Duration,
    /// Longest pause between retries.
    max: Duration,
    /// Longest one record is retried.
    budget: Duration,
    /// Longest a flush lets the records before it keep trying.
    drain: Duration,
    /// Longest a flush waits past `drain` for the write in flight.
    settle: Duration,
}

impl RecordRetry {
    /// The service's retry: [`RECORD_FIRST_RETRY`], [`RECORD_MAX_RETRY`],
    /// [`RECORD_RETRY_BUDGET`], [`RECORD_DRAIN_BOUND`] and
    /// [`RECORD_FLUSH_SETTLE`].
    pub const fn service() -> Self {
        Self {
            first: RECORD_FIRST_RETRY,
            max: RECORD_MAX_RETRY,
            budget: RECORD_RETRY_BUDGET,
            drain: RECORD_DRAIN_BOUND,
            settle: RECORD_FLUSH_SETTLE,
        }
    }
}
