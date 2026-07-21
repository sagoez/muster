use nutype::nutype;

/// A process's display name: trimmed and non-empty.
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
pub struct ProcessName(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_surrounding_whitespace() {
        let name = ProcessName::try_new("  Claude Code  ").unwrap();
        assert_eq!(name.as_ref(), "Claude Code");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!(ProcessName::try_new("").is_err());
        assert!(ProcessName::try_new("   ").is_err());
    }
}
