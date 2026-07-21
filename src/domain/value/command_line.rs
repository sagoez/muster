use nutype::nutype;

/// The shell command used to launch a process: trimmed and non-empty.
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
pub struct CommandLine(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_inner_spacing_but_trims_edges() {
        let cmd = CommandLine::try_new("  npm run dev  ").unwrap();
        assert_eq!(cmd.as_ref(), "npm run dev");
    }

    #[test]
    fn rejects_blank() {
        assert!(CommandLine::try_new("   ").is_err());
    }
}
