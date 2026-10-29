use std::time::SystemTime;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{event_kind::EventKind, event_seq::EventSeq};
use crate::domain::value::{OutputCursor, ProcessName, ProjectRoot};

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
