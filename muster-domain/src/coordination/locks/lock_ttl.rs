use std::time::Duration;

use nutype::nutype;

/// The time-to-live a lock takes when the agent names none.
pub const LOCK_TTL_DEFAULT: Duration = Duration::from_secs(5 * 60);
/// The longest time-to-live a lock may take.
pub const LOCK_TTL_MAX: Duration = Duration::from_secs(60 * 60);

/// Whether `ttl` is a whole second or more and at most [`LOCK_TTL_MAX`].
const fn is_lock_ttl(ttl: &Duration) -> bool {
    ttl.as_secs() >= 1 && ttl.as_secs() <= LOCK_TTL_MAX.as_secs()
}

/// How long a lock holds before it frees itself: at least a second, at most
/// [`LOCK_TTL_MAX`].
#[nutype(
    const_fn,
    validate(predicate = is_lock_ttl),
    derive(Debug, Clone, Copy, PartialEq, Eq, AsRef)
)]
pub struct LockTtl(Duration);

impl LockTtl {
    /// The time-to-live of [`LOCK_TTL_DEFAULT`], checked at compile time.
    pub const DEFAULT: Self = match Self::try_new(LOCK_TTL_DEFAULT) {
        Ok(ttl) => ttl,
        Err(_) => panic!("the default lock ttl is out of range"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ttl is at least a second and at most the maximum.
    #[test]
    fn a_ttl_is_bounded() {
        assert!(LockTtl::try_new(Duration::ZERO).is_err());
        assert!(LockTtl::try_new(LOCK_TTL_MAX).is_ok());
        assert!(LockTtl::try_new(LOCK_TTL_MAX + Duration::from_secs(1)).is_err());
        assert_eq!(*LockTtl::DEFAULT.as_ref(), LOCK_TTL_DEFAULT);
    }
}
