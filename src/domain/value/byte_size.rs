use std::{
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use bytesize::ByteSize as RawByteSize;
use nutype::nutype;

use super::ValueParseError;

/// The form a byte size is written in.
const SIZE_FORM: &str = "a positive size such as 10MiB";
/// The binary units a size is written in, largest first, each with its
/// bytes.
const UNITS: [(&str, u64); 6] = [
    ("EiB", RawByteSize::eib(1).as_u64()),
    ("PiB", RawByteSize::pib(1).as_u64()),
    ("TiB", RawByteSize::tib(1).as_u64()),
    ("GiB", RawByteSize::gib(1).as_u64()),
    ("MiB", RawByteSize::mib(1).as_u64()),
    ("KiB", RawByteSize::kib(1).as_u64()),
];
/// The unit of a size that no larger unit divides.
const BYTES_UNIT: &str = "B";

/// A positive quantity of bytes, read from strings such as `"10MiB"`.
#[nutype(
    validate(predicate = |size: &RawByteSize| size.as_u64() > 0),
    derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)
)]
pub struct ByteSize(RawByteSize);

impl ByteSize {
    /// The quantity as a plain byte count.
    pub fn bytes(&self) -> u64 {
        self.into_inner().as_u64()
    }
}

impl Display for ByteSize {
    /// The size exactly, in the largest binary unit that divides it, or in
    /// bytes, so the text reads back as the same size.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let bytes = self.bytes();
        let (unit, per) = UNITS
            .into_iter()
            .find(|(_, per)| bytes.is_multiple_of(*per))
            .unwrap_or((BYTES_UNIT, 1));
        write!(f, "{}{unit}", bytes / per)
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

    /// What a size displays as reads back as the same size, a size no unit
    /// divides included, which shows in bytes.
    #[test]
    fn display_round_trips_through_from_str() {
        for (raw, shown) in [
            ("10MiB", "10MiB"),
            ("1000001", "1000001B"),
            ("1536KiB", "1536KiB"),
            ("1 GB", "1000000000B"),
            ("2048", "2KiB"),
        ] {
            let size: ByteSize = raw.parse().unwrap();
            assert_eq!(size.to_string(), shown, "{raw}");
            assert_eq!(size.to_string().parse::<ByteSize>().unwrap(), size, "{raw}");
        }
        assert!("lots".parse::<ByteSize>().is_err());
    }
}
