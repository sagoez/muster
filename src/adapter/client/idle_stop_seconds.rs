//! Serde `with` encoding of an idle stop in `--json` and MCP output: whole
//! seconds, rounded up, or null for never.

use serde::{Serialize, Serializer};

use super::whole_seconds_up;
use crate::domain::value::IdleStop;

/// `idle_stop` in whole seconds, rounded up, or none when it never comes.
pub fn seconds(idle_stop: &IdleStop) -> Option<u64> {
    idle_stop.after().map(whole_seconds_up)
}

/// Writes `idle_stop` as whole seconds, or null for never.
///
/// # Errors
/// Returns the serializer's error.
pub fn serialize<S: Serializer>(idle_stop: &IdleStop, serializer: S) -> Result<S::Ok, S::Error> {
    seconds(idle_stop).serialize(serializer)
}

/// Writes an optional idle stop as whole seconds, or null for never or none.
///
/// # Errors
/// Returns the serializer's error.
pub fn serialize_optional<S: Serializer>(
    idle_stop: &Option<IdleStop>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    idle_stop.as_ref().and_then(seconds).serialize(serializer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duration is whole seconds with any part of one counted, and never
    /// is none.
    #[test]
    fn an_idle_stop_is_whole_seconds_or_none() {
        let seconds_of = |raw: &str| seconds(&raw.parse().unwrap());

        assert_eq!(seconds_of("30m"), Some(1800));
        assert_eq!(seconds_of("1500ms"), Some(2));
        assert_eq!(seconds_of("never"), None);
    }
}
