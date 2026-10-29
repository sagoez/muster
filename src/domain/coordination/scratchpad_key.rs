use nutype::nutype;

/// Most characters a note's key holds.
pub const NOTE_KEY_MAX_CHARS: usize = 128;

/// Name identifying a scratchpad within a project. Trimmed, non-empty and at
/// most [`NOTE_KEY_MAX_CHARS`] long, so a blank key can never address a note
/// and one key cannot flood a listing.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = NOTE_KEY_MAX_CHARS),
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
pub struct ScratchpadKey(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A key is trimmed of surrounding whitespace.
    #[test]
    fn a_key_is_trimmed() {
        assert_eq!(ScratchpadKey::try_new("  plan  ").unwrap().as_ref(), "plan");
    }

    /// A blank or whitespace-only key is rejected.
    #[test]
    fn a_blank_key_is_rejected() {
        assert!(ScratchpadKey::try_new("   ").is_err());
        assert!(ScratchpadKey::try_new("").is_err());
    }

    /// A key up to the maximum is kept; one past it is refused.
    #[test]
    fn a_key_is_bounded() {
        assert!(ScratchpadKey::try_new("k".repeat(NOTE_KEY_MAX_CHARS)).is_ok());
        assert_eq!(
            ScratchpadKey::try_new("k".repeat(NOTE_KEY_MAX_CHARS + 1)),
            Err(ScratchpadKeyError::LenCharMaxViolated)
        );
    }
}
