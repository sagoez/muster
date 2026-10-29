use std::{fmt, str::FromStr, time::SystemTime};

use getset::{CopyGetters, Getters};
use nutype::nutype;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::value::{OutputCursor, ProcessName, ProjectRoot, ValueParseError};

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

/// What happened to a process, as the event log records it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    /// A run started.
    Started,
    /// A run passed its readiness check.
    Ready,
    /// A run did not pass its readiness check in time.
    ReadyTimeout,
    /// A run ended with status 0, or failed while being stopped.
    Exited,
    /// A run failed on its own.
    Crashed,
    /// A run ended and its restart policy will start it again.
    Restarting,
    /// A run was stopped on request.
    Stopped,
    /// A start was refused because something else holds what it needs.
    Blocked,
    /// A warning or error notice was raised.
    Notice,
}

/// One entry of the service's event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
pub struct StackEvent {
    /// Its position in the log.
    #[getset(get_copy = "pub")]
    seq: EventSeq,
    /// When it happened.
    #[getset(get = "pub")]
    #[serde(with = "humantime_serde")]
    at: SystemTime,
    /// The project it concerns, or none for the service as a whole.
    #[getset(get = "pub")]
    project: Option<ProjectRoot>,
    /// The process it concerns, when one.
    #[getset(get = "pub")]
    process: Option<ProcessName>,
    /// What happened.
    #[getset(get_copy = "pub")]
    kind: EventKind,
    /// More about it, for a person to read.
    #[getset(get = "pub")]
    detail: String,
    /// Where the process's output stood when it happened.
    #[getset(get = "pub")]
    cursor: Option<OutputCursor>,
}

/// Events after a cursor, and the cursor to continue from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct EventPage {
    /// The events, oldest first.
    events: Vec<StackEvent>,
    /// The newest position in the log, to pass as `since` next time.
    cursor: EventSeq,
    /// Whether events after `since` were dropped before they were read.
    gap: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seq_reads_and_writes_its_text_form() {
        assert_eq!("e:9921".parse::<EventSeq>().unwrap(), EventSeq::new(9921));
        assert_eq!(EventSeq::new(7).to_string(), "e:7");
        assert!("9921".parse::<EventSeq>().is_err());
    }

    #[test]
    fn kinds_are_kebab_case() {
        assert_eq!(EventKind::ReadyTimeout.to_string(), "ready-timeout");
        assert_eq!("crashed".parse::<EventKind>().unwrap(), EventKind::Crashed);
    }
}
