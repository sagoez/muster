use std::str::FromStr;

use bytesize::ByteSize as RawByteSize;
use nutype::nutype;

use super::ValueParseError;

/// The form a byte size is written in.
const SIZE_FORM: &str = "a positive size such as 10MiB";

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

impl FromStr for ByteSize {
    type Err = ValueParseError;

    /// Reads a size such as `10MiB` or `512 KB`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for unreadable text or zero bytes.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .parse::<RawByteSize>()
            .ok()
            .and_then(|size| Self::try_new(size).ok())
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(SIZE_FORM)
                    .found(raw)
                    .build()
            })
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
        assert!("0B".parse::<ByteSize>().is_err());
    }

    /// What a size displays as reads back as the same size.
    #[test]
    fn display_round_trips_through_from_str() {
        let size: ByteSize = "10MiB".parse().unwrap();

        assert_eq!(size.to_string().parse::<ByteSize>().unwrap(), size);
        assert!("lots".parse::<ByteSize>().is_err());
    }
}
