use getset::Getters;
use serde::Serialize;

use super::{PageSize, TodoView};
use crate::domain::coordination::{Todo, TodoId};

/// One page of a todo listing: the todos, whether more follow, and the id to
/// resume after when they do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters)]
#[getset(get = "pub")]
pub struct TodoPage {
    /// The todos on this page.
    todos: Vec<TodoView>,
    /// Whether the listing holds more past this page.
    more: bool,
    /// The last todo's id when more follow, to pass as the next cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<TodoId>,
}

impl TodoPage {
    /// The first `size` of `listed`, saying whether more follow.
    pub fn of(listed: impl IntoIterator<Item = TodoView>, size: PageSize) -> Self {
        let mut listed = listed.into_iter();
        let todos: Vec<TodoView> = listed.by_ref().take(size.into_inner()).collect();
        let more = listed.next().is_some();
        let cursor = more
            .then(|| todos.last().map(|last| last.id().clone()))
            .flatten();
        Self {
            todos,
            more,
            cursor,
        }
    }

    /// Where a listing of `todos` resumes after the todo `after` names: just
    /// past it, or the start with no cursor. `None` when no todo has that id.
    pub fn resume(todos: &[Todo], after: Option<&TodoId>) -> Option<usize> {
        match after {
            None => Some(0),
            Some(id) => todos
                .iter()
                .position(|todo| todo.id() == id)
                .map(|at| at + 1),
        }
    }
}
