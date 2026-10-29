use std::time::Duration;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// An agent's hold on an in-progress todo: who works it, and the Unix second
/// its lease runs out.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoClaim {
    /// The agent working the todo.
    owner: Author,
    /// Unix epoch seconds after which the lease has run out.
    claimed_until: u64,
}

impl TodoClaim {
    /// Whether the lease still holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: u64) -> bool {
        self.claimed_until >= now
    }

    /// How long the lease has left at `now`; zero once it has run out.
    #[must_use]
    pub fn expires_in(&self, now: u64) -> Duration {
        Duration::from_secs(self.claimed_until.saturating_sub(now))
    }
}
