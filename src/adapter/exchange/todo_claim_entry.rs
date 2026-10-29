use serde::{Deserialize, Serialize};

use crate::{
    adapter::wire::{WireForm, form},
    domain::coordination::{Author, TodoClaim},
};

/// An agent's hold on a todo, as a coordination snapshot keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoClaimEntry {
    /// The agent working the todo.
    #[serde(with = "form")]
    owner: Author,
    /// Unix epoch seconds after which the lease has run out.
    claimed_until: u64,
}

impl WireForm for TodoClaim {
    type Wire = TodoClaimEntry;

    fn to_wire(&self) -> TodoClaimEntry {
        TodoClaimEntry {
            owner: self.owner().clone(),
            claimed_until: *self.claimed_until(),
        }
    }

    fn from_wire(wire: TodoClaimEntry) -> Result<Self, String> {
        Ok(Self::builder()
            .owner(wire.owner)
            .claimed_until(wire.claimed_until)
            .build())
    }
}
