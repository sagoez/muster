use std::time::Duration;

use nutype::nutype;

/// The lease a claim takes when the agent names none.
pub const TODO_LEASE_DEFAULT: Duration = Duration::from_secs(30 * 60);
/// The longest lease a claim may take.
pub const TODO_LEASE_MAX: Duration = Duration::from_secs(8 * 60 * 60);

/// Whether `lease` is a whole second or more and at most [`TODO_LEASE_MAX`].
const fn is_lease(lease: &Duration) -> bool {
    lease.as_secs() >= 1 && lease.as_secs() <= TODO_LEASE_MAX.as_secs()
}

/// How long a todo claim holds before another agent may take the todo over:
/// at least a second, at most [`TODO_LEASE_MAX`].
#[nutype(
    const_fn,
    validate(predicate = is_lease),
    derive(Debug, Clone, Copy, PartialEq, Eq, AsRef)
)]
pub struct TodoLease(Duration);

impl TodoLease {
    /// The lease of [`TODO_LEASE_DEFAULT`], checked at compile time.
    pub const DEFAULT: Self = match Self::try_new(TODO_LEASE_DEFAULT) {
        Ok(lease) => lease,
        Err(_) => panic!("the default todo lease is out of range"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lease is at least a second and at most the maximum.
    #[test]
    fn a_lease_is_bounded() {
        assert!(TodoLease::try_new(Duration::ZERO).is_err());
        assert!(TodoLease::try_new(Duration::from_millis(999)).is_err());
        assert!(TodoLease::try_new(TODO_LEASE_MAX).is_ok());
        assert!(TodoLease::try_new(TODO_LEASE_MAX + Duration::from_secs(1)).is_err());
        assert_eq!(*TodoLease::DEFAULT.as_ref(), TODO_LEASE_DEFAULT);
    }
}
