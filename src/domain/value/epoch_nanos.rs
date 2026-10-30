use std::time::{SystemTime, UNIX_EPOCH};

use nutype::nutype;

/// A moment as nanoseconds since the Unix epoch, as precise as its clock
/// reads: when a project was registered, or when a shell drew a prompt.
#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Display,
    FromStr,
    Serialize,
    Deserialize
))]
pub struct EpochNanos(u64);

impl EpochNanos {
    /// The moment `time` names, held at the epoch before it and at the
    /// latest representable nanosecond past that.
    pub fn of(time: SystemTime) -> Self {
        let nanos = time
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        Self::new(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A moment is read to the nanosecond, and one before the epoch is the
    /// epoch.
    #[test]
    fn a_moment_counts_nanoseconds_from_the_epoch() {
        let moment = UNIX_EPOCH + Duration::new(2, 5);

        assert_eq!(EpochNanos::of(moment), EpochNanos::new(2_000_000_005));
        assert_eq!(
            EpochNanos::of(UNIX_EPOCH - Duration::from_secs(1)),
            EpochNanos::new(0)
        );
    }
}
