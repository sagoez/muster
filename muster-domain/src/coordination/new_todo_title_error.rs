use thiserror::Error;

use super::new_todo_title::TODO_TITLE_MAX_CHARS;

/// Why a title cannot be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NewTodoTitleError {
    /// The title is blank once trimmed.
    #[error("a todo title cannot be blank")]
    Blank,
    /// The title runs past [`TODO_TITLE_MAX_CHARS`].
    #[error("a todo title holds at most {TODO_TITLE_MAX_CHARS} characters")]
    TooLong,
}
