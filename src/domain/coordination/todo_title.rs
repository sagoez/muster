use nutype::nutype;

/// Most characters a todo's title holds.
pub const TODO_TITLE_MAX_CHARS: usize = 500;

/// A todo's human-readable title. Trimmed, non-empty and at most
/// [`TODO_TITLE_MAX_CHARS`] long, so a blank line can never masquerade as a
/// task and one title cannot flood a listing.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = TODO_TITLE_MAX_CHARS),
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
pub struct TodoTitle(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A title up to the maximum is kept; one past it is refused.
    #[test]
    fn a_title_is_bounded() {
        assert!(TodoTitle::try_new("x".repeat(TODO_TITLE_MAX_CHARS)).is_ok());
        assert_eq!(
            TodoTitle::try_new("x".repeat(TODO_TITLE_MAX_CHARS + 1)),
            Err(TodoTitleError::LenCharMaxViolated)
        );
    }
}
