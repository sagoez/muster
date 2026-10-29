use crate::domain::coordination::Author;

/// What releasing a lock came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockRelease {
    /// The caller held the lock and it is gone.
    Released,
    /// Someone else holds it, so it stays.
    HeldBy {
        /// Who holds it.
        owner: Author,
    },
    /// Nobody holds it.
    NotHeld,
}
