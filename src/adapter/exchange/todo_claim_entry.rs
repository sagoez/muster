use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::{
    adapter::wire::{WireForm, form},
    domain::{
        coordination::{Author, TodoClaim},
        value::EpochSeconds,
    },
};

/// An agent's hold on a todo, as a coordination snapshot keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(deny_unknown_fields)]
pub struct TodoClaimEntry {
    /// The agent working the todo.
    #[serde(with = "form")]
    owner: Author,
    /// Unix epoch seconds after which the lease has run out.
    claimed_until: EpochSeconds,
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
