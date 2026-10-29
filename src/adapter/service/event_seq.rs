use std::{fmt, str::FromStr};

use nutype::nutype;

use crate::domain::value::ValueParseError;

/// Prefix of an event cursor's text form.
const SEQ_PREFIX: &str = "e:";
/// The form an event cursor is written in.
const SEQ_FORM: &str = "an event cursor such as e:9921";

/// The position of an event in the service's event log, counting every event
/// ever recorded. Written as `e:<seq>`.
#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize
))]
pub struct EventSeq(u64);

impl fmt::Display for EventSeq {
    /// Writes `e:<seq>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{SEQ_PREFIX}{}", self.into_inner())
    }
}

impl FromStr for EventSeq {
    type Err = ValueParseError;

    /// Reads `e:<seq>`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for any other text.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .strip_prefix(SEQ_PREFIX)
            .and_then(|seq| seq.parse().ok())
            .map(Self::new)
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(SEQ_FORM)
                    .found(raw)
                    .build()
            })
    }
}
