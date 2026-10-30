use std::time::Duration;

use crate::domain::coordination::{Author, Todo};

/// What an attempt to claim a todo came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The caller now holds the todo, or renewed its lease.
    Claimed(Todo),
    /// Another agent holds the todo under a live lease.
    HeldBy {
        /// The agent holding it.
        owner: Author,
        /// How long its lease has left.
        expires_in: Duration,
    },
    /// The named todo cannot be claimed: it is done, or in progress with no
    /// agent's lease on it.
    Unavailable(Todo),
    /// No todo was named and none is claimable.
    NothingClaimable,
    /// No todo has the named id.
    Missing,
}
