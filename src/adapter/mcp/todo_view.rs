use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, ClaimState, Todo, TodoId, TodoStatus, TodoTitle};

/// One todo as an agent reads it, with who holds it. An expired claim reads
/// as pending, its holder shown as `stale_owner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoView {
    /// Its id.
    id: TodoId,
    /// What the task is.
    title: TodoTitle,
    /// Where it sits in its lifecycle.
    status: TodoStatus,
    /// Ids of todos it depends on.
    deps: Vec<TodoId>,
    /// Who holds it under a live claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<Author>,
    /// Seconds until that claim lapses.
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_s: Option<u64>,
    /// Who held it under a claim that lapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    stale_owner: Option<Author>,
    /// Who last wrote it.
    author: Author,
    /// When it was last written, in seconds since the Unix epoch.
    updated_at: u64,
}

impl TodoView {
    /// `todo` as read at `now`, Unix seconds.
    pub fn at(todo: &Todo, now: u64) -> Self {
        let (owner, expires_in_s, stale_owner) = match todo.claim_state(now) {
            ClaimState::Unclaimed => (None, None, None),
            ClaimState::Held { owner, expires_in } => {
                (Some(owner.clone()), Some(expires_in.as_secs()), None)
            },
            ClaimState::Stale { owner } => (None, None, Some(owner.clone())),
        };
        Self {
            id: todo.id().clone(),
            title: todo.title().clone(),
            status: todo.effective_status(now),
            deps: todo.deps().clone(),
            owner,
            expires_in_s,
            stale_owner,
            author: todo.author().clone(),
            updated_at: *todo.updated_at(),
        }
    }
}
