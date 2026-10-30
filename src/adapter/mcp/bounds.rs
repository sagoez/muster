//! How long a call may block and how much it may return, so every call
//! answers within what MCP clients allow and fits an agent's context. A
//! value outside a bound is refused, never quietly clamped.

use std::time::Duration;

use super::{process_fields::invalid, tool_error::ToolError};
use crate::adapter::client::{MAX_LIST_LIMIT, PageSize};

/// Longest any call blocks, under the 60 seconds many MCP clients give one.
pub const CALL_BOUND: Duration = Duration::from_secs(45);
/// How long a wait runs when the call does not say.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);
/// How often a call that carries a progress token hears that it still runs.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
/// The field a page size is given as.
const LIMIT_FIELD: &str = "limit";
/// The fields a wait is given as: how long a call waits for its condition,
/// and how long it waits for a lock or an event.
pub const TIMEOUT_FIELD: &str = "timeout_s";
pub const WAIT_S_FIELD: &str = "wait_s";
/// The form a wait is written in, followed by [`CALL_BOUND`].
const WAIT_FORM: &str = "whole seconds from 0 to ";
/// The form a page size is written in, followed by [`MAX_LIST_LIMIT`].
const PAGE_FORM: &str = "a number from 1 to ";

/// The wait a call asked for as `field` in whole seconds, `default` when it
/// did not.
///
/// # Errors
/// Returns the caller's mistake for a wait past [`CALL_BOUND`].
pub fn wait_bound(
    field: &'static str,
    seconds: Option<u64>,
    default: Duration,
) -> Result<Duration, ToolError> {
    match seconds {
        None => Ok(default),
        Some(seconds) if seconds <= CALL_BOUND.as_secs() => Ok(Duration::from_secs(seconds)),
        Some(seconds) => Err(invalid(
            field,
            &format!("{WAIT_FORM}{}", CALL_BOUND.as_secs()),
            &seconds.to_string(),
        )),
    }
}

/// The page size a call asked for, the default size when it did not.
///
/// # Errors
/// Returns the caller's mistake for zero or a size past [`MAX_LIST_LIMIT`].
pub fn page_size(limit: Option<usize>) -> Result<PageSize, ToolError> {
    limit.map_or(Ok(PageSize::default()), |limit| {
        PageSize::try_new(limit).map_err(|_| {
            invalid(
                LIMIT_FIELD,
                &format!("{PAGE_FORM}{MAX_LIST_LIMIT}"),
                &limit.to_string(),
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wait defaults, and one past the call bound is refused, never cut.
    #[test]
    fn a_wait_defaults_and_past_the_bound_is_refused() {
        assert_eq!(
            wait_bound("timeout_s", None, DEFAULT_WAIT).unwrap(),
            DEFAULT_WAIT
        );
        assert_eq!(
            wait_bound("timeout_s", Some(3), DEFAULT_WAIT).unwrap(),
            Duration::from_secs(3)
        );
        assert_eq!(
            wait_bound("wait_s", Some(CALL_BOUND.as_secs()), Duration::ZERO).unwrap(),
            CALL_BOUND
        );
        let refused = wait_bound("timeout_s", Some(600), DEFAULT_WAIT)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("timeout_s") && refused.contains("600"),
            "{refused}"
        );
    }

    /// A page size defaults; zero and one past the cap are refused.
    #[test]
    fn a_page_size_defaults_and_out_of_range_is_refused() {
        assert_eq!(page_size(None).unwrap(), PageSize::default());
        assert_eq!(page_size(Some(5)).unwrap().into_inner(), 5);
        assert!(page_size(Some(0)).is_err());
        assert!(page_size(Some(MAX_LIST_LIMIT + 1)).is_err());
    }
}
