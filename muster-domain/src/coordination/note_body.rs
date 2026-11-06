use nutype::nutype;

/// Most characters a note's body holds.
pub const NOTE_BODY_MAX_CHARS: usize = 65_536;

/// A shared note's markdown body, at most [`NOTE_BODY_MAX_CHARS`] long, so
/// one write cannot fill every agent's context that reads it.
#[nutype(
    validate(len_char_max = NOTE_BODY_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct NoteBody(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A body up to the maximum is kept; one past it is refused.
    #[test]
    fn a_note_body_is_bounded() {
        assert!(NoteBody::try_new("x".repeat(NOTE_BODY_MAX_CHARS)).is_ok());
        assert!(NoteBody::try_new("x".repeat(NOTE_BODY_MAX_CHARS + 1)).is_err());
    }
}
