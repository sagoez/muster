use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::value::EpochSeconds;

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
