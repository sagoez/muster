use nutype::nutype;

use crate::coordination::NOTE_BODY_MAX_CHARS;

/// The start of a note's body, as a short read takes it: never longer than
/// a body may be.
#[nutype(
    validate(len_char_max = NOTE_BODY_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct NoteBodyStart(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A start no longer than a body is kept; a longer one is refused.
    #[test]
    fn a_body_start_is_no_longer_than_a_body() {
        assert!(NoteBodyStart::try_new("x".repeat(NOTE_BODY_MAX_CHARS)).is_ok());
        assert!(NoteBodyStart::try_new("x".repeat(NOTE_BODY_MAX_CHARS + 1)).is_err());
    }
}
