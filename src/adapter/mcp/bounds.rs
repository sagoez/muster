//! How long a call may block and how much it may return, so every call
//! answers within what MCP clients allow and fits an agent's context.

use std::time::Duration;

/// Longest any call blocks, under the 60 seconds many MCP clients give one.
pub const CALL_BOUND: Duration = Duration::from_secs(45);
/// How long a wait runs when the call does not say.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);
/// How often a call that carries a progress token hears that it still runs.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
/// Entries a list tool returns when the caller does not ask for a size.
pub const DEFAULT_LIST_LIMIT: usize = 50;
/// Hard cap on a list tool's page.
pub const MAX_LIST_LIMIT: usize = 200;

/// The wait a call asked for in whole seconds, `DEFAULT_WAIT` when it did
/// not, never past [`CALL_BOUND`].
pub fn wait_bound(seconds: Option<u64>) -> Duration {
    seconds
        .map_or(DEFAULT_WAIT, Duration::from_secs)
        .min(CALL_BOUND)
}

/// A requested page size within the allowed range.
pub fn page_size(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wait defaults, and never passes the call bound.
    #[test]
    fn a_wait_defaults_and_stays_under_the_bound() {
        assert_eq!(wait_bound(None), DEFAULT_WAIT);
        assert_eq!(wait_bound(Some(3)), Duration::from_secs(3));
        assert_eq!(wait_bound(Some(600)), CALL_BOUND);
    }

    /// A page size defaults, clamps to the cap, and never yields an empty
    /// page.
    #[test]
    fn page_size_defaults_and_clamps() {
        assert_eq!(page_size(None), DEFAULT_LIST_LIMIT);
        assert_eq!(page_size(Some(5)), 5);
        assert_eq!(page_size(Some(0)), 1, "a zero page would return nothing");
        assert_eq!(page_size(Some(10_000)), MAX_LIST_LIMIT);
    }
}
