use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::{
    adapter::wire::{WireForm, form, optional_form},
    domain::{
        coordination::{
            Author, TODO_DEPS_MAX, Todo, TodoClaim, TodoDeps, TodoId, TodoStatus, TodoTitle,
        },
        value::EpochSeconds,
    },
};

/// A todo as a coordination snapshot keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
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
    /// When it was last written, as RFC 3339 UTC.
    #[serde(with = "form")]
    updated_at: EpochSeconds,
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
        let found = wire.deps.len();
        let deps = TodoDeps::try_new(wire.deps).map_err(|_| {
            format!(
                "todo '{}' depends on {found} todos; at most {TODO_DEPS_MAX} are allowed",
                wire.id.as_ref()
            )
        })?;
        Ok(Self::builder()
            .id(wire.id)
            .title(wire.title)
            .status(wire.status)
            .deps(deps.into_inner())
            .claim(wire.claim)
            .author(wire.author)
            .updated_at(wire.updated_at)
            .build())
    }
}
