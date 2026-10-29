use std::time::Duration;

use crate::domain::coordination::{Author, LockNote};

/// What an attempt to take a lock came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockAttempt {
    /// The caller holds the lock, newly or renewed.
    Acquired {
        /// How long it holds before it expires.
        expires_in: Duration,
    },
    /// Someone else holds it.
    HeldBy {
        /// Who holds it.
        owner: Author,
        /// What they said they hold it for.
        note: Option<LockNote>,
        /// How long it has left.
        expires_in: Duration,
    },
}
