use std::time::Duration;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, LockName, LockNote};

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
    /// Unix epoch seconds after which the lock has expired.
    expires_at: u64,
}

impl Lock {
    /// Whether the lock still holds at `now` (Unix seconds).
    #[must_use]
    pub fn is_live(&self, now: u64) -> bool {
        self.expires_at >= now
    }

    /// How long the lock has left at `now`; zero once it has expired.
    #[must_use]
    pub fn expires_in(&self, now: u64) -> Duration {
        Duration::from_secs(self.expires_at.saturating_sub(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lock is live through its last second and reports what it has left.
    #[test]
    fn a_lock_expires_after_its_last_second() {
        let lock = Lock::builder()
            .name(LockName::try_new("migrations").unwrap())
            .owner(Author::agent("claude"))
            .note(None)
            .expires_at(100)
            .build();
        assert!(lock.is_live(100));
        assert!(!lock.is_live(101));
        assert_eq!(lock.expires_in(40), Duration::from_secs(60));
        assert_eq!(lock.expires_in(200), Duration::ZERO);
    }
}
