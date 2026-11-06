use super::{new_todo_title_error::NewTodoTitleError, todo_title::TodoTitle};

/// Most characters a todo's title holds when it is written.
pub const TODO_TITLE_MAX_CHARS: usize = 500;

/// A title being written to a todo: a [`TodoTitle`] at most
/// [`TODO_TITLE_MAX_CHARS`] long, so one write cannot flood a listing. Titles
/// already stored keep whatever length they have. Hand-written, since nutype
/// cannot narrow another newtype.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTodoTitle(TodoTitle);

impl NewTodoTitle {
    /// The title `raw`, trimmed.
    ///
    /// # Errors
    /// Returns [`NewTodoTitleError::Blank`] for a blank title and
    /// [`NewTodoTitleError::TooLong`] for one past [`TODO_TITLE_MAX_CHARS`].
    pub fn try_new(raw: impl Into<String>) -> Result<Self, NewTodoTitleError> {
        let title = TodoTitle::try_new(raw).map_err(|_| NewTodoTitleError::Blank)?;
        if title.as_ref().chars().count() > TODO_TITLE_MAX_CHARS {
            return Err(NewTodoTitleError::TooLong);
        }
        Ok(Self(title))
    }

    /// The title as a todo keeps it.
    pub fn title(&self) -> &TodoTitle {
        &self.0
    }
}

impl From<NewTodoTitle> for TodoTitle {
    fn from(title: NewTodoTitle) -> Self {
        title.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title up to the maximum is written; one past it or a blank one is
    /// refused.
    #[test]
    fn a_written_title_is_bounded() {
        assert!(NewTodoTitle::try_new("x".repeat(TODO_TITLE_MAX_CHARS)).is_ok());
        assert_eq!(
            NewTodoTitle::try_new("x".repeat(TODO_TITLE_MAX_CHARS + 1)),
            Err(NewTodoTitleError::TooLong)
        );
        assert_eq!(NewTodoTitle::try_new("   "), Err(NewTodoTitleError::Blank));
    }

    /// The title is trimmed as a stored title is.
    #[test]
    fn a_written_title_is_trimmed() {
        assert_eq!(
            NewTodoTitle::try_new("  ship  ").unwrap().title().as_ref(),
            "ship"
        );
    }
}
