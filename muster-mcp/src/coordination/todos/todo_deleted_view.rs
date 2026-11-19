use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `todo_delete` answers: the todo named, and whether there was one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct TodoDeletedView {
    /// The todo's id.
    #[getset(get = "pub")]
    id: String,
    /// Whether a todo was deleted; false when there was none.
    #[getset(get_copy = "pub")]
    deleted: bool,
}
