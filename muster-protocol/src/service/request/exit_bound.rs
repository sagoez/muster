use std::time::Duration;

/// Longest the service takes to exit once asked, before it answers the
/// client that asked: the wait for launches in flight, then the flush of
/// what it records, then the flush of the notices it mirrors to its log.
/// A client's stop waits this long; the service asserts its parts sum to it.
pub const SERVICE_EXIT_BOUND: Duration = Duration::from_secs(30);
