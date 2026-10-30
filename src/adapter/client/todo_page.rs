use getset::Getters;
use serde::{Serialize, Serializer};
use typed_builder::TypedBuilder;

use super::{PageSize, TodoView};
use crate::domain::coordination::{EntryPosition, Todo};

/// One page of a todo listing: the todos, whether more follow, and the
/// position to resume after when they do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoPage {
    /// The todos on this page.
    todos: Vec<TodoView>,
    /// Whether the listing holds more past this page.
    more: bool,
    /// The last todo's position when more follow, to pass as the next
    /// cursor.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "cursor_text"
    )]
    cursor: Option<EntryPosition>,
}

/// `cursor` as the text an agent passes back, as note and key-value cursors
/// are.
fn cursor_text<S: Serializer>(
    cursor: &Option<EntryPosition>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match cursor {
        Some(position) => serializer.collect_str(position),
        None => serializer.serialize_none(),
    }
}

impl TodoPage {
    /// The first `size` of `listed`, saying whether more follow.
    pub fn of(listed: impl IntoIterator<Item = (EntryPosition, TodoView)>, size: PageSize) -> Self {
        let mut listed = listed.into_iter();
        let page: Vec<(EntryPosition, TodoView)> =
            listed.by_ref().take(size.into_inner()).collect();
        let more = listed.next().is_some();
        let cursor = more
            .then(|| page.last().map(|(position, _)| *position))
            .flatten();
        Self {
            todos: page.into_iter().map(|(_, view)| view).collect(),
            more,
            cursor,
        }
    }

    /// Where a listing of `todos` resumes after the position `after`: at the
    /// first todo past it, so a deleted todo never breaks the listing, or the
    /// start with no cursor.
    pub fn resume(todos: &[(EntryPosition, Todo)], after: Option<EntryPosition>) -> usize {
        after.map_or(0, |after| {
            todos.partition_point(|(position, _)| *position <= after)
        })
    }
}
