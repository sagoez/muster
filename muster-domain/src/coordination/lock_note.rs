use nutype::nutype;

/// The longest note a holder may leave on a lock, in characters.
const LOCK_NOTE_MAX_CHARS: usize = 200;

/// Whether `note` holds no control character, so it prints as one line.
fn is_one_line(note: &str) -> bool {
    !note.chars().any(char::is_control)
}

/// What a holder says it holds a lock for: trimmed, one line, at most 200
/// characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = LOCK_NOTE_MAX_CHARS, predicate = is_one_line),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct LockNote(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A note is one bounded line.
    #[test]
    fn a_lock_note_is_one_bounded_line() {
        assert!(LockNote::try_new("running migrations").is_ok());
        assert!(LockNote::try_new("two\nlines").is_err());
        assert!(LockNote::try_new("\u{1b}[31mred").is_err());
        assert!(LockNote::try_new("x".repeat(LOCK_NOTE_MAX_CHARS + 1)).is_err());
    }
}
