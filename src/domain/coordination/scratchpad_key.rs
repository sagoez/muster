use nutype::nutype;

/// Name identifying a scratchpad within a project. Trimmed and non-empty, so a
/// blank or whitespace-only key can never address a note.
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
}
