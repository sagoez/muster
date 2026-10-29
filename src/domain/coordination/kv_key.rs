use nutype::nutype;

/// Name addressing a shared key-value entry within a project. Trimmed and
/// non-empty, so a blank key can never address a value.
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
pub struct KvKey(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A key is trimmed; a blank key is rejected.
    #[test]
    fn a_key_is_trimmed_and_blank_is_rejected() {
        assert_eq!(KvKey::try_new("  port  ").unwrap().as_ref(), "port");
        assert!(KvKey::try_new("   ").is_err());
    }
}
