use std::{fmt, time::Duration};

use serde::{Serialize, Serializer};

/// Milliseconds in a second, to write an elapsed time as seconds.
const MILLIS_PER_SECOND: u128 = 1000;
/// Milliseconds as a float, to serialize seconds with millisecond precision.
const MILLIS_PER_SECOND_F64: f64 = 1000.0;

/// How long a wait took, as seconds with millisecond precision: `2.14` in
/// JSON for the CLI and MCP alike, and `2.140s` for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElapsedSeconds(Duration);

impl From<Duration> for ElapsedSeconds {
    /// The elapsed time `duration`, read to the millisecond.
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl fmt::Display for ElapsedSeconds {
    /// Writes `2.140s`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let millis = self.0.as_millis();
        write!(
            formatter,
            "{}.{:03}s",
            millis / MILLIS_PER_SECOND,
            millis % MILLIS_PER_SECOND
        )
    }
}

impl Serialize for ElapsedSeconds {
    /// Serializes the seconds as a number with at most three decimals.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let millis = (self.0.as_secs_f64() * MILLIS_PER_SECOND_F64).trunc();
        serializer.serialize_f64(millis / MILLIS_PER_SECOND_F64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An elapsed time reads as seconds to the millisecond, for a person and
    /// in JSON, dropping what is below a millisecond.
    #[test]
    fn an_elapsed_time_is_seconds_to_the_millisecond() {
        let elapsed = ElapsedSeconds::from(Duration::from_micros(62_140_900));

        assert_eq!(elapsed.to_string(), "62.140s");
        assert_eq!(serde_json::to_string(&elapsed).unwrap(), "62.14");
        assert_eq!(
            ElapsedSeconds::from(Duration::from_millis(5)).to_string(),
            "0.005s"
        );
    }
}
