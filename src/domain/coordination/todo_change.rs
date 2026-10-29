use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::coordination::{NewTodoTitle, TodoStatus};

/// A change to one todo: a new title, a new status, or both. Handing a todo
/// back is the status [`TodoStatus::Pending`], which drops its claim.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct TodoChange {
    /// The new title, or `None` to keep it.
    #[getset(get = "pub")]
    title: Option<NewTodoTitle>,
    /// The new status, or `None` to keep it.
    #[getset(get_copy = "pub")]
    status: Option<TodoStatus>,
    /// Whether to change a todo another agent holds under a live lease.
    #[getset(get_copy = "pub")]
    force: bool,
}
