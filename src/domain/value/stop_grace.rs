use std::{fmt, str::FromStr, time::Duration};

use nutype::nutype;

use super::ValueParseError;

/// The longest grace a graceful stop waits before it kills.
pub const MAX_STOP_GRACE: Duration = Duration::from_secs(60 * 60);
/// The form a stop grace is written in.
const STOP_GRACE_FORM: &str = "a positive duration up to 1h, such as 5s or 1m 30s";

/// Whether `duration` is longer than zero and at most [`MAX_STOP_GRACE`].
const fn is_grace(duration: &Duration) -> bool {
    !duration.is_zero() && duration.as_nanos() <= MAX_STOP_GRACE.as_nanos()
}

/// How long a graceful stop waits before it kills: longer than zero and at
/// most [`MAX_STOP_GRACE`]. Its constructor is `const`, so a constant one is
/// checked at compile time.
#[nutype(
    const_fn,
    validate(predicate = is_grace),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct StopGrace(Duration);

impl FromStr for StopGrace {
    type Err = ValueParseError;

    /// Reads a humantime duration such as `5s`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for unreadable text, zero, or more than
    /// [`MAX_STOP_GRACE`].
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        humantime::parse_duration(raw.trim())
            .ok()
            .and_then(|duration| Self::try_new(duration).ok())
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(STOP_GRACE_FORM)
                    .found(raw)
                    .build()
            })
    }
}

impl fmt::Display for StopGrace {
    /// Writes the grace in the humantime form it is read from.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}",
            humantime::format_duration(self.into_inner())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grace is positive and at most an hour, whichever way it arrives.
    #[test]
    fn a_grace_is_positive_and_at_most_an_hour() {
        assert!(StopGrace::try_new(Duration::ZERO).is_err());
        assert!(StopGrace::try_new(MAX_STOP_GRACE).is_ok());
        assert!(StopGrace::try_new(MAX_STOP_GRACE + Duration::from_nanos(1)).is_err());
        assert!("1h 1s".parse::<StopGrace>().is_err());
        assert!("300000000y".parse::<StopGrace>().is_err());
        assert!(serde_json::from_str::<StopGrace>(r#"{"secs":3601,"nanos":0}"#).is_err());
    }

    #[test]
    fn reads_and_writes_humantime() {
        let grace: StopGrace = "1m 30s".parse().unwrap();

        assert_eq!(grace.into_inner(), Duration::from_secs(90));
        assert_eq!(grace.to_string(), "1m 30s");
        assert!("0s".parse::<StopGrace>().is_err());
        assert!("soon".parse::<StopGrace>().is_err());
    }
}
