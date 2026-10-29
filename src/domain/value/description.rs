use nutype::nutype;

/// Optional one-line description of a process: trimmed and non-empty. Wrap in `Option` for the absent case.
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
}
