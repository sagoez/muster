use std::{fmt, str::FromStr};

use nutype::nutype;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use super::ValueParseError;

/// What every cursor's text starts with.
const CURSOR_PREFIX: &str = "o:";
/// The form a cursor is written in.
const CURSOR_FORM: &str = "an output cursor such as o:18234";

/// The absolute byte offset of a position in a process's output: the
/// concatenation of every run of that process as written to its log, across
/// rotations and restarts. Its text is opaque to agents: `o:<offset>`.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash))]
pub struct OutputCursor(u64);

impl OutputCursor {
    /// The absolute byte offset.
    pub fn offset(&self) -> u64 {
        self.into_inner()
    }

    /// The cursor `bytes` further on, saturating at the largest offset.
    pub fn advanced(&self, bytes: u64) -> Self {
        Self::new(self.offset().saturating_add(bytes))
    }

    /// Bytes from `earlier` to this cursor, or zero when `earlier` is not
    /// earlier.
    pub fn bytes_since(&self, earlier: Self) -> u64 {
        self.offset().saturating_sub(earlier.offset())
    }
}

impl fmt::Display for OutputCursor {
    /// Writes the cursor as `o:<offset>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{CURSOR_PREFIX}{}", self.offset())
    }
}

impl FromStr for OutputCursor {
    type Err = ValueParseError;

    /// Reads a cursor written by [`fmt::Display`].
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for text without the prefix or a decimal
    /// offset.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .strip_prefix(CURSOR_PREFIX)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|digits| digits.parse().ok())
            .map(Self::new)
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(CURSOR_FORM)
                    .found(raw)
                    .build()
            })
    }
}

impl Serialize for OutputCursor {
    /// Serializes the cursor as its opaque text.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for OutputCursor {
    /// Reads the cursor from its opaque text.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cursor's text reads back as the same cursor, on its own and in JSON.
    #[test]
    fn a_cursor_round_trips_through_its_text() {
        let cursor = OutputCursor::new(18234);
        assert_eq!(cursor.to_string(), "o:18234");
        assert_eq!("o:18234".parse::<OutputCursor>().unwrap(), cursor);

        let json = serde_json::to_string(&cursor).unwrap();
        assert_eq!(json, r#""o:18234""#);
        assert_eq!(serde_json::from_str::<OutputCursor>(&json).unwrap(), cursor);
    }

    /// Text that is not a cursor is refused with the accepted form.
    #[test]
    fn malformed_cursors_are_refused() {
        for raw in [
            "18234",
            "o:",
            "o:-1",
            "o:+5",
            "e:5",
            "o:1x",
            "o:99999999999999999999",
        ] {
            assert!(raw.parse::<OutputCursor>().is_err(), "{raw}");
        }
        assert!(serde_json::from_str::<OutputCursor>("18234").is_err());
        assert_eq!(
            "x".parse::<OutputCursor>().unwrap_err().to_string(),
            "expected an output cursor such as o:18234, got 'x'"
        );
    }

    #[test]
    fn cursors_advance_and_measure_saturating() {
        let cursor = OutputCursor::new(10);
        assert_eq!(cursor.advanced(5).offset(), 15);
        assert_eq!(OutputCursor::new(u64::MAX).advanced(1).offset(), u64::MAX);
        assert_eq!(cursor.advanced(5).bytes_since(cursor), 5);
        assert_eq!(cursor.bytes_since(cursor.advanced(5)), 0);
    }
}
