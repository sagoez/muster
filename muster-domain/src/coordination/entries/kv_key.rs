use nutype::nutype;

/// Most characters a key-value key holds.
pub const KV_KEY_MAX_CHARS: usize = 128;

/// Name addressing a shared key-value entry within a project. Trimmed,
/// non-empty and at most [`KV_KEY_MAX_CHARS`] long, so a blank key can never
/// address a value and one key cannot flood a listing.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = KV_KEY_MAX_CHARS),
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

    /// A key up to the maximum is kept; one past it is refused.
    #[test]
    fn a_key_is_bounded() {
        assert!(KvKey::try_new("k".repeat(KV_KEY_MAX_CHARS)).is_ok());
        assert_eq!(
            KvKey::try_new("k".repeat(KV_KEY_MAX_CHARS + 1)),
            Err(KvKeyError::LenCharMaxViolated)
        );
    }
}
