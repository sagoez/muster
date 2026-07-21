use nutype::nutype;

/// A registered project's display name: trimmed and non-empty.
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
pub struct ProjectName(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_rejects_empty() {
        assert_eq!(
            ProjectName::try_new("  muster  ").unwrap().as_ref(),
            "muster"
        );
        assert!(ProjectName::try_new("   ").is_err());
    }
}
