use nutype::nutype;

use crate::coordination::NOTE_BODY_MAX_CHARS;

/// The most bytes a character takes in UTF-8.
const MAX_CHAR_BYTES: usize = 4;
/// The most bytes a note's body can take.
pub const NOTE_BODY_MAX_BYTES: usize = NOTE_BODY_MAX_CHARS * MAX_CHAR_BYTES;

/// How many bytes a whole note body takes: never more than a body of the
/// most characters could.
#[nutype(
    validate(less_or_equal = NOTE_BODY_MAX_BYTES),
    derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display)
)]
pub struct NoteBodyBytes(usize);

#[cfg(test)]
mod tests {
    use super::*;

    /// A length a body could take is kept; a longer one is refused.
    #[test]
    fn a_body_length_is_one_a_body_could_take() {
        assert!(NoteBodyBytes::try_new(NOTE_BODY_MAX_BYTES).is_ok());
        assert!(NoteBodyBytes::try_new(NOTE_BODY_MAX_BYTES + 1).is_err());
    }
}
