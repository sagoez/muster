use std::time::Duration;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    coordination::{Author, TodoLease},
    value::EpochSeconds,
};

/// An agent's hold on an in-progress todo: who works it, and the moment
/// its lease runs out.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoClaim {
    /// The agent working the todo.
    owner: Author,
    /// The moment after which the lease has run out.
    claimed_until: EpochSeconds,
}

impl TodoClaim {
    /// A claim for `owner` taken at `now` for `lease`.
    #[must_use]
    pub fn fresh(owner: &Author, lease: TodoLease, now: EpochSeconds) -> Self {
        Self {
            owner: owner.clone(),
            claimed_until: now.after(*lease.as_ref()),
        }
    }

    /// Whether the lease still holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: EpochSeconds) -> bool {
        self.claimed_until >= now
    }

    /// How long the lease has left at `now`; zero once it has run out.
    #[must_use]
    pub fn expires_in(&self, now: EpochSeconds) -> Duration {
        now.until(self.claimed_until)
    }
}
