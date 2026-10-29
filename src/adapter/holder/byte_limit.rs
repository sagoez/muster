use nutype::nutype;

/// The most bytes of text one log read returns.
pub const MAX_LOG_BYTES: usize = 64 * 1024;
/// Bytes of text a log read returns when the caller names no limit.
const DEFAULT_LOG_BYTES: usize = 16 * 1024;

/// How many bytes of text a log read returns at most: 1 to
/// [`MAX_LOG_BYTES`], counting each line and its line feed.
#[nutype(
    validate(greater = 0, less_or_equal = MAX_LOG_BYTES),
    default = DEFAULT_LOG_BYTES,
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, Default, Serialize, Deserialize)
)]
pub struct ByteLimit(usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_limit_is_bounded_and_defaults_to_16_kib() {
        assert_eq!(ByteLimit::default().into_inner(), DEFAULT_LOG_BYTES);
        assert!(ByteLimit::try_new(0).is_err());
        assert!(ByteLimit::try_new(MAX_LOG_BYTES).is_ok());
        assert!(ByteLimit::try_new(MAX_LOG_BYTES + 1).is_err());
    }
}
