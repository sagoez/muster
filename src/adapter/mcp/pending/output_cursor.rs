use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::domain::value::ValueParseError;

/// Prefix of an output cursor on the wire.
const OUTPUT_PREFIX: &str = "o:";
/// The form an output cursor is written in.
const OUTPUT_CURSOR_FORM: &str = "a cursor such as o:18234, from an earlier result";

/// Absolute byte offset into a process's output across every run. Stand-in
/// for package A's `domain/value/output_cursor.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OutputCursor(u64);

impl OutputCursor {
    /// The cursor at `offset`.
    pub const fn at(offset: u64) -> Self {
        Self(offset)
    }
}

impl fmt::Display for OutputCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{OUTPUT_PREFIX}{}", self.0)
    }
}

impl FromStr for OutputCursor {
    type Err = ValueParseError;

    /// Reads `o:<offset>`.
    ///
    /// # Errors
    /// Returns the form expected when `raw` is not one.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .strip_prefix(OUTPUT_PREFIX)
            .and_then(|offset| offset.parse().ok())
            .map(Self)
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(OUTPUT_CURSOR_FORM)
                    .found(raw)
                    .build()
            })
    }
}
