use std::time::{Duration, UNIX_EPOCH};

use super::WireForm;
use crate::domain::value::EpochSeconds;

/// The last second RFC 3339 can write, 9999-12-31T23:59:59Z; a later moment
/// is written as this one rather than failing.
const LATEST_RFC3339_SECOND: u64 = 253_402_300_799;
/// Ends an RFC 3339 time in UTC.
const UTC_SUFFIX: &str = "Z";
/// Characters of an RFC 3339 offset such as `+02:00`.
const OFFSET_LEN: usize = 6;
/// What a time is written as when its offset is out of form.
const OFFSET_FORM: &str = "expected an RFC 3339 time ending in Z or an offset such as +02:00";
/// Hours an offset stays below.
const HOURS_PER_DAY: i64 = 24;
/// Minutes in an hour, which an offset's minutes stay below.
const MINUTES_PER_HOUR: i64 = 60;
/// Seconds in a minute.
const SECONDS_PER_MINUTE: i64 = 60;

impl WireForm for EpochSeconds {
    type Wire = String;

    /// The moment as RFC 3339 UTC to the second.
    fn to_wire(&self) -> String {
        let seconds = self.into_inner().min(LATEST_RFC3339_SECOND);
        humantime::format_rfc3339_seconds(UNIX_EPOCH + Duration::from_secs(seconds)).to_string()
    }

    /// The moment any RFC 3339 time names, in UTC or at an offset, its
    /// fraction of a second dropped.
    fn from_wire(wire: String) -> Result<Self, String> {
        let upper = wire.to_ascii_uppercase();
        let (utc, offset) = at_utc(&upper).ok_or_else(|| format!("{OFFSET_FORM}, got {wire}"))?;
        let at = humantime::parse_rfc3339(&utc).map_err(|error| error.to_string())?;
        let since = at
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?;
        since
            .as_secs()
            .checked_add_signed(-offset)
            .map(Self::new)
            .ok_or_else(|| format!("{wire} is before 1970"))
    }
}

/// `time` with its offset replaced by `Z`, and the offset in seconds east
/// of UTC; none for an offset out of `[+-]HH:MM` form.
fn at_utc(time: &str) -> Option<(String, i64)> {
    if time.ends_with(UTC_SUFFIX) {
        return Some((time.to_string(), 0));
    }
    let split = time.len().checked_sub(OFFSET_LEN)?;
    let (local, offset) = (time.get(..split)?, time.get(split..)?);
    let sign = match offset.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let (hours, minutes) = offset.get(1..)?.split_once(':')?;
    let (hours, minutes): (i64, i64) = (two_digits(hours)?, two_digits(minutes)?);
    if hours >= HOURS_PER_DAY || minutes >= MINUTES_PER_HOUR {
        return None;
    }
    Some((
        format!("{local}{UTC_SUFFIX}"),
        sign * (hours * MINUTES_PER_HOUR + minutes) * SECONDS_PER_MINUTE,
    ))
}

/// The number two ASCII digits write.
fn two_digits(text: &str) -> Option<i64> {
    (text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
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

    /// Any RFC 3339 time reads, normalised to the UTC second: a fraction is
    /// dropped and an offset is applied.
    #[test]
    fn any_rfc3339_time_reads_as_its_utc_second() {
        let utc = EpochSeconds::from_wire("2026-10-29T16:42:59Z".to_string()).unwrap();

        for written in [
            "2026-10-29T16:42:59.7Z",
            "2026-10-29T18:42:59+02:00",
            "2026-10-29T18:42:59.123456+02:00",
            "2026-10-29T14:12:59-02:30",
            "2026-10-29t16:42:59z",
            "2026-10-29T16:42:59+00:00",
        ] {
            assert_eq!(
                EpochSeconds::from_wire(written.to_string()),
                Ok(utc),
                "{written}"
            );
        }
        for refused in [
            "2026-10-29T18:42:59+2:00",
            "2026-10-29T18:42:59+02",
            "2026-10-29 18:42:59",
            "2026-10-29T18:42:59+24:00",
        ] {
            assert!(
                EpochSeconds::from_wire(refused.to_string()).is_err(),
                "{refused}"
            );
        }
    }
}
