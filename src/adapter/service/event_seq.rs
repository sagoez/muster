use std::{fmt, str::FromStr};

use getset::CopyGetters;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use typed_builder::TypedBuilder;

use crate::domain::value::ValueParseError;

/// Prefix of an event cursor's text form.
const SEQ_PREFIX: &str = "e:";
/// Separates an event cursor's epoch from its count.
const EPOCH_SEPARATOR: char = ':';
/// The form an event cursor is written in.
const SEQ_FORM: &str = "an event cursor such as e:3141592:9921";

/// The position of an event in one service's event log: the epoch naming
/// that service, and how many events it had recorded. Written as
/// `e:<epoch>:<seq>`, so a cursor from an earlier service is told apart from
/// one of the current log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct EventSeq {
    /// The service whose log numbered the event.
    epoch: u32,
    /// How many events that log had recorded, counting this one.
    seq: u64,
}

impl EventSeq {
    /// Whether this cursor names a later event than `other` of the same
    /// log; never for a cursor of another log.
    pub fn is_after(&self, other: Self) -> bool {
        self.epoch == other.epoch && self.seq > other.seq
    }
}

impl fmt::Display for EventSeq {
    /// Writes `e:<epoch>:<seq>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{SEQ_PREFIX}{}{EPOCH_SEPARATOR}{}",
            self.epoch, self.seq
        )
    }
}

impl FromStr for EventSeq {
    type Err = ValueParseError;

    /// Reads `e:<epoch>:<seq>`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for any other text.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .strip_prefix(SEQ_PREFIX)
            .and_then(|rest| rest.split_once(EPOCH_SEPARATOR))
            .and_then(|(epoch, seq)| Some((epoch.parse().ok()?, seq.parse().ok()?)))
            .map(|(epoch, seq)| Self { epoch, seq })
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(SEQ_FORM)
                    .found(raw)
                    .build()
            })
    }
}

impl Serialize for EventSeq {
    /// Serializes the cursor as its text.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EventSeq {
    /// Reads the cursor from its text.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cursor's text names its epoch and count and reads back the same;
    /// the old single-number form is refused.
    #[test]
    fn a_cursor_reads_back_from_its_text_and_json() {
        let cursor = EventSeq::builder().epoch(31).seq(9921).build();

        assert_eq!(cursor.to_string(), "e:31:9921");
        assert_eq!("e:31:9921".parse::<EventSeq>().unwrap(), cursor);
        assert_eq!(
            serde_json::from_str::<EventSeq>(&serde_json::to_string(&cursor).unwrap()).unwrap(),
            cursor
        );
        assert!("e:9921".parse::<EventSeq>().is_err());
        assert!("9921".parse::<EventSeq>().is_err());
    }

    /// Only a later event of the same log is after a cursor.
    #[test]
    fn a_cursor_of_another_epoch_is_never_after() {
        let seen = EventSeq::builder().epoch(1).seq(5).build();

        assert!(EventSeq::builder().epoch(1).seq(6).build().is_after(seen));
        assert!(!EventSeq::builder().epoch(1).seq(5).build().is_after(seen));
        assert!(!EventSeq::builder().epoch(2).seq(90).build().is_after(seen));
    }
}
