use nutype::nutype;

/// Most characters a key-value entry's value holds.
pub const KV_VALUE_MAX_CHARS: usize = 4_096;

/// A key-value entry's value, a small fact at most [`KV_VALUE_MAX_CHARS`]
/// long.
#[nutype(
    validate(len_char_max = KV_VALUE_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct KvValue(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A value up to the maximum is kept; one past it is refused.
    #[test]
    fn a_value_is_bounded() {
        assert!(KvValue::try_new("x".repeat(KV_VALUE_MAX_CHARS)).is_ok());
        assert!(KvValue::try_new("x".repeat(KV_VALUE_MAX_CHARS + 1)).is_err());
    }
}
