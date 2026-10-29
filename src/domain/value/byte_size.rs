use bytesize::ByteSize as RawByteSize;
use nutype::nutype;

/// A positive quantity of bytes, read from strings such as `"10MiB"`.
#[nutype(
    validate(predicate = |size: &RawByteSize| size.as_u64() > 0),
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, Serialize, Deserialize)
)]
pub struct ByteSize(RawByteSize);

impl ByteSize {
    /// The quantity as a plain byte count.
    pub fn bytes(&self) -> u64 {
        self.into_inner().as_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_human_readable_size() {
        let size: ByteSize = serde_yaml_ng::from_str("10MiB").unwrap();
        assert_eq!(size.bytes(), 10 * 1024 * 1024);
    }

    #[test]
    fn rejects_zero() {
        assert!(serde_yaml_ng::from_str::<ByteSize>("0B").is_err());
        assert!(ByteSize::try_new(RawByteSize::b(0)).is_err());
    }
}
