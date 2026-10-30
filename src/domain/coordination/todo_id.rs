use nutype::nutype;
use uuid::Uuid;

/// Most characters a todo id holds; a generated id is a 36-character UUID.
pub const TODO_ID_MAX_CHARS: usize = 64;

/// Stable identity of a todo within a project, generated when the todo is
/// added, at most [`TODO_ID_MAX_CHARS`] long.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = TODO_ID_MAX_CHARS),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct TodoId(String);

impl TodoId {
    /// Creates a globally unique identity for a new todo.
    ///
    /// # Errors
    /// Returns the generated newtype's validation error if its invariant ever
    /// diverges from UUID's non-empty textual representation.
    pub fn generate() -> Result<Self, TodoIdError> {
        Self::try_new(Uuid::new_v4().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A generated id fits; an id past the maximum is refused.
    #[test]
    fn an_id_is_bounded() {
        assert!(TodoId::generate().is_ok());
        assert!(TodoId::try_new("i".repeat(TODO_ID_MAX_CHARS)).is_ok());
        assert_eq!(
            TodoId::try_new("i".repeat(TODO_ID_MAX_CHARS + 1)),
            Err(TodoIdError::LenCharMaxViolated)
        );
    }
}
