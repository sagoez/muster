use serde::{Deserialize, Serialize};

use crate::{
    adapter::wire::{WireForm, form, optional_form},
    domain::coordination::{Author, Todo, TodoClaim, TodoId, TodoStatus, TodoTitle},
};

/// A todo as a coordination snapshot keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoEntry {
    /// Its stable identity.
    id: TodoId,
    /// What the task is.
    title: TodoTitle,
    /// Where it sits in its lifecycle.
    #[serde(with = "form")]
    status: TodoStatus,
    /// Ids of todos it depends on.
    deps: Vec<TodoId>,
    /// The agent working it and until when.
    #[serde(with = "optional_form")]
    claim: Option<TodoClaim>,
    /// Who last wrote it.
    #[serde(with = "form")]
    author: Author,
    /// Unix epoch seconds of the last write.
    updated_at: u64,
}

impl WireForm for Todo {
    type Wire = TodoEntry;

    fn to_wire(&self) -> TodoEntry {
        TodoEntry {
            id: self.id().clone(),
            title: self.title().clone(),
            status: *self.status(),
            deps: self.deps().clone(),
            claim: self.claim().clone(),
            author: self.author().clone(),
            updated_at: *self.updated_at(),
        }
    }

    fn from_wire(wire: TodoEntry) -> Result<Self, String> {
        Ok(Self::builder()
            .id(wire.id)
            .title(wire.title)
            .status(wire.status)
            .deps(wire.deps)
            .claim(wire.claim)
            .author(wire.author)
            .updated_at(wire.updated_at)
            .build())
    }
}
