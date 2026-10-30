use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::value::{EpochNanos, EpochSeconds};

/// The current moment, or the epoch itself if the clock predates it. The one
/// clock the store stamps coordination writes with and the CLI and MCP read
/// leases and locks against.
pub fn epoch_now() -> EpochSeconds {
    EpochSeconds::new(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    )
}

/// The current moment to the nanosecond, on the same clock as
/// [`epoch_now`]: what a registration is stamped with, and what a shell's
/// report is timed by when its hook gives no prompt time.
pub fn nanos_now() -> EpochNanos {
    EpochNanos::of(SystemTime::now())
}
