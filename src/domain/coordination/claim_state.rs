use std::time::Duration;

use crate::domain::coordination::Author;

/// Who holds a todo at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState<'a> {
    /// Nobody claimed it.
    Unclaimed,
    /// An agent holds it under a live lease.
    Held {
        /// The agent holding it.
        owner: &'a Author,
        /// How long the lease has left.
        expires_in: Duration,
    },
    /// An agent claimed it and let the lease run out.
    Stale {
        /// The agent whose lease ran out.
        owner: &'a Author,
    },
}
