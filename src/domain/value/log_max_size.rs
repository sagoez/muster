use std::str::FromStr;

use bytesize::ByteSize as RawByteSize;
use nutype::nutype;

use super::{ByteSize, ValueParseError};

/// The smallest size at which a process's log may rotate.
pub const MIN_LOG_MAX_SIZE: RawByteSize = RawByteSize::kib(16);
/// The largest size at which a process's log may rotate.
pub const MAX_LOG_MAX_SIZE: RawByteSize = RawByteSize::gib(1);
/// The form a log size is written in.
const LOG_SIZE_FORM: &str = "a size from 16KiB to 1GiB such as 10MiB";

/// Whether `size` lies between [`MIN_LOG_MAX_SIZE`] and [`MAX_LOG_MAX_SIZE`].
fn is_log_size(size: &ByteSize) -> bool {
    (MIN_LOG_MAX_SIZE.as_u64()..=MAX_LOG_MAX_SIZE.as_u64()).contains(&size.bytes())
}

/// The size at which a process's log rotates, between [`MIN_LOG_MAX_SIZE`]
/// and [`MAX_LOG_MAX_SIZE`].
#[nutype(
    validate(predicate = is_log_size),
    derive(Debug, Clone, Copy, PartialEq, Eq, Display)
)]
pub struct LogMaxSize(ByteSize);

impl LogMaxSize {
    /// The size as a plain byte count.
    pub fn bytes(&self) -> u64 {
        self.into_inner().bytes()
    }
}

impl FromStr for LogMaxSize {
    type Err = ValueParseError;

    /// Reads a size such as `10MiB`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for unreadable text or a size out of
    /// bounds.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.parse::<ByteSize>()
            .ok()
            .and_then(|size| Self::try_new(size).ok())
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(LOG_SIZE_FORM)
                    .found(raw)
                    .build()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_size_is_bounded_both_ways() {
        assert_eq!(
            "10MiB".parse::<LogMaxSize>().unwrap().bytes(),
            10 * 1024 * 1024
        );
        assert!("16KiB".parse::<LogMaxSize>().is_ok());
        assert!("1GiB".parse::<LogMaxSize>().is_ok());
        assert!("4KiB".parse::<LogMaxSize>().is_err());
        assert!("2GiB".parse::<LogMaxSize>().is_err());
        assert!("16EiB".parse::<LogMaxSize>().is_err());
    }
}
