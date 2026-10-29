use nutype::nutype;

/// A todo's human-readable title: trimmed and non-empty, so a blank line can
/// never masquerade as a task. A title being written is capped by
/// [`NewTodoTitle`](crate::domain::coordination::NewTodoTitle); a stored one is
/// read back whatever its length.
#[nutype(
    sanitize(trim),
    validate(not_empty),
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
    use crate::domain::coordination::TODO_TITLE_MAX_CHARS;

    /// A stored title past the write cap still reads back.
    #[test]
    fn a_stored_title_is_not_capped() {
        assert!(TodoTitle::try_new("x".repeat(TODO_TITLE_MAX_CHARS + 1)).is_ok());
    }
}
