use nutype::nutype;

/// Most characters a process's description holds.
pub const DESCRIPTION_MAX_CHARS: usize = 300;

/// Optional one-line description of a process: trimmed, non-empty and at most
/// [`DESCRIPTION_MAX_CHARS`] long. Wrap in `Option` for the absent case.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = DESCRIPTION_MAX_CHARS),
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
pub struct Description(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank() {
        assert!(Description::try_new("  ").is_err());
    }

    #[test]
    fn accepts_a_sentence() {
        let d = Description::try_new("A CLI interface for the project").unwrap();
        assert_eq!(d.as_ref(), "A CLI interface for the project");
    }

    /// A description up to the maximum is kept; one past it is refused.
    #[test]
    fn a_description_is_bounded() {
        assert!(Description::try_new("x".repeat(DESCRIPTION_MAX_CHARS)).is_ok());
        assert_eq!(
            Description::try_new("x".repeat(DESCRIPTION_MAX_CHARS + 1)),
            Err(DescriptionError::LenCharMaxViolated)
        );
    }
}
