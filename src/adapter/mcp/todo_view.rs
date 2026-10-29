use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::pending::lease_of;
use crate::domain::coordination::{Author, Todo, TodoId, TodoStatus, TodoTitle};

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
        let lease = lease_of(todo);
        let live = lease.as_ref().filter(|lease| lease.claimed_until() > now);
        let stale = lease.as_ref().filter(|lease| lease.claimed_until() <= now);
        Self {
            id: todo.id().clone(),
            title: todo.title().clone(),
            status: if stale.is_some() {
                TodoStatus::Pending
            } else {
                *todo.status()
            },
            deps: todo.deps().clone(),
            owner: live.map(|lease| lease.owner().clone()),
            expires_in_s: live.map(|lease| lease.claimed_until() - now),
            stale_owner: stale.map(|lease| lease.owner().clone()),
            author: todo.author().clone(),
            updated_at: *todo.updated_at(),
        }
    }
}
