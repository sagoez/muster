use std::time::Duration;

use nutype::nutype;

/// Whether `duration` is longer than zero.
const fn is_positive(duration: &Duration) -> bool {
    !duration.is_zero()
}

/// A duration longer than zero. Its constructor is `const`, so a constant one
/// is checked at compile time.
#[nutype(
    const_fn,
    validate(predicate = is_positive),
    derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, AsRef)
)]
pub struct PositiveDuration(Duration);

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
}
