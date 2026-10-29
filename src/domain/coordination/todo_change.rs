use std::time::Duration;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, Todo, TodoStatus, TodoTitle};

/// A change to one todo: a new title, a new status, or both. Handing a todo
/// back is the status [`TodoStatus::Pending`], which drops its claim.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct TodoChange {
    /// The new title, or `None` to keep it.
    #[getset(get = "pub")]
    title: Option<TodoTitle>,
    /// The new status, or `None` to keep it.
    #[getset(get_copy = "pub")]
    status: Option<TodoStatus>,
    /// Whether to change a todo another agent holds under a live lease.
    #[getset(get_copy = "pub")]
    force: bool,
}

/// What a todo change came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoUpdate {
    /// The todo as stored after the change.
    Updated(Todo),
    /// Another agent holds the todo under a live lease and the change did not
    /// force.
    HeldBy {
        /// The agent holding it.
        owner: Author,
        /// How long its lease has left.
        expires_in: Duration,
    },
    /// No todo has the id.
    Missing,
}
