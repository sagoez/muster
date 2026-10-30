use std::time::Duration;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    coordination::{Author, LockName, LockNote},
    value::EpochSeconds,
};

/// A named lock one agent holds on a project until it releases it or it
/// expires.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Lock {
    /// The lock's name.
    name: LockName,
    /// Who holds it.
    owner: Author,
    /// What the holder said it holds it for.
    note: Option<LockNote>,
    /// The moment after which the lock has expired.
    expires_at: EpochSeconds,
}

impl Lock {
    /// Whether the lock still holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: EpochSeconds) -> bool {
        self.expires_at >= now
    }

    /// How long the lock has left at `now`; zero once it has expired.
    #[must_use]
    pub fn expires_in(&self, now: EpochSeconds) -> Duration {
        now.until(self.expires_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The moment `secs` seconds after the epoch.
    fn at(secs: u64) -> EpochSeconds {
        EpochSeconds::new(secs)
    }

    /// A lock is live through its last second and reports what it has left.
    #[test]
    fn a_lock_expires_after_its_last_second() {
        let lock = Lock::builder()
            .name(LockName::try_new("migrations").unwrap())
            .owner(Author::agent("claude"))
            .note(None)
            .expires_at(at(100))
            .build();
        assert!(lock.is_live(at(100)));
        assert!(!lock.is_live(at(101)));
        assert_eq!(lock.expires_in(at(40)), Duration::from_secs(60));
        assert_eq!(lock.expires_in(at(200)), Duration::ZERO);
    }
}
