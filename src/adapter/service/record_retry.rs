use std::time::Duration;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Pause before the first retry of a record that failed to store.
pub const RECORD_FIRST_RETRY: Duration = Duration::from_millis(100);
/// Longest pause between retries of a record that keeps failing.
pub const RECORD_MAX_RETRY: Duration = Duration::from_secs(5);
/// Tries a record gets, the first included, before it is given up: with the
/// backoff above, about half a minute of a database that refuses it.
pub const RECORD_ATTEMPTS: u32 = 10;

/// How a [`StoreRecorder`](super::StoreRecorder) retries a record the store
/// refuses: the pause doubles from `first` up to `max`, and after `attempts`
/// tries the record is given up so later ones can land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct RecordRetry {
    /// Pause before the first retry.
    first: Duration,
    /// Longest pause between retries.
    max: Duration,
    /// Tries, the first included, before the record is given up.
    attempts: u32,
}

impl RecordRetry {
    /// The service's retry: [`RECORD_FIRST_RETRY`], [`RECORD_MAX_RETRY`] and
    /// [`RECORD_ATTEMPTS`].
    pub const fn service() -> Self {
        Self {
            first: RECORD_FIRST_RETRY,
            max: RECORD_MAX_RETRY,
            attempts: RECORD_ATTEMPTS,
        }
    }
}
