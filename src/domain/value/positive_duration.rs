use std::{fmt, str::FromStr, time::Duration};

use nutype::nutype;

use super::ValueParseError;

/// The form a duration is written in.
const DURATION_FORM: &str = "a positive duration such as 5s or 1m 30s";

/// Whether `duration` is longer than zero.
const fn is_positive(duration: &Duration) -> bool {
    !duration.is_zero()
}

/// A duration longer than zero. Its constructor is `const`, so a constant one
/// is checked at compile time.
#[nutype(
    const_fn,
    validate(predicate = is_positive),
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
pub struct PositiveDuration(Duration);

impl FromStr for PositiveDuration {
    type Err = ValueParseError;

    /// Reads a humantime duration such as `5s`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for unreadable text or a zero duration.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        humantime::parse_duration(raw.trim())
            .ok()
            .and_then(|duration| Self::try_new(duration).ok())
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(DURATION_FORM)
                    .found(raw)
                    .build()
            })
    }
}

impl fmt::Display for PositiveDuration {
    /// Writes the duration in the humantime form it is read from.
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

    #[test]
    fn refuses_a_zero_duration() {
        assert!(PositiveDuration::try_new(Duration::ZERO).is_err());
        let one = Duration::from_nanos(1);
        assert_eq!(
            PositiveDuration::try_new(one).map(PositiveDuration::into_inner),
            Ok(one)
        );
    }

    #[test]
    fn reads_and_writes_humantime() {
        let grace: PositiveDuration = "1m 30s".parse().unwrap();

        assert_eq!(grace.into_inner(), Duration::from_secs(90));
        assert_eq!(grace.to_string(), "1m 30s");
        assert!("0s".parse::<PositiveDuration>().is_err());
        assert!("soon".parse::<PositiveDuration>().is_err());
    }
}
