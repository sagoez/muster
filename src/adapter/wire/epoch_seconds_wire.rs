use std::time::{Duration, UNIX_EPOCH};

use super::WireForm;
use crate::domain::value::EpochSeconds;

/// The last second RFC 3339 can write, 9999-12-31T23:59:59Z; a later moment
/// is written as this one rather than failing.
const LATEST_RFC3339_SECOND: u64 = 253_402_300_799;

impl WireForm for EpochSeconds {
    type Wire = String;

    /// The moment as RFC 3339 UTC to the second.
    fn to_wire(&self) -> String {
        let seconds = self.into_inner().min(LATEST_RFC3339_SECOND);
        humantime::format_rfc3339_seconds(UNIX_EPOCH + Duration::from_secs(seconds)).to_string()
    }

    /// The moment an RFC 3339 UTC time names, its fraction of a second
    /// dropped.
    fn from_wire(wire: String) -> Result<Self, String> {
        let at = humantime::parse_rfc3339(&wire).map_err(|error| error.to_string())?;
        let since = at
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?;
        Ok(Self::new(since.as_secs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A moment is written as RFC 3339 and read back to the same second.
    #[test]
    fn a_moment_round_trips_as_rfc3339() {
        let at = EpochSeconds::new(1_793_288_081);

        assert_eq!(at.to_wire(), "2026-10-29T15:34:41Z");
        assert_eq!(EpochSeconds::from_wire(at.to_wire()).unwrap(), at);
    }

    /// A moment past what RFC 3339 can write is held at its last second,
    /// and text that is no RFC 3339 time is refused.
    #[test]
    fn out_of_range_moments_are_held_and_bad_text_refused() {
        assert_eq!(
            EpochSeconds::new(u64::MAX).to_wire(),
            "9999-12-31T23:59:59Z"
        );
        assert!(EpochSeconds::from_wire("1793288081".to_string()).is_err());
    }
}
