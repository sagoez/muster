use std::{
    fmt,
    str::FromStr,
    time::{Duration, SystemTime},
};

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};
use typed_builder::TypedBuilder;

use super::output_cursor::OutputCursor;
use crate::{
    adapter::{client::ServiceClient, mcp::tool_error::ToolError},
    domain::{
        project::Project,
        value::{ProcessName, ValueParseError},
    },
};

/// Prefix of an event cursor on the wire.
const EVENT_PREFIX: &str = "e:";
/// The form an event cursor is written in.
const EVENT_CURSOR_FORM: &str = "a cursor such as e:9921, from an earlier result";
/// The feature these stand-ins wait for.
const EVENTS: &str = "stack events (package B)";

/// Sequence number of one stack event. Stand-in for package B's `EventSeq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventSeq(u64);

impl EventSeq {
    /// The event numbered `seq`.
    pub const fn at(seq: u64) -> Self {
        Self(seq)
    }
}

impl fmt::Display for EventSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{EVENT_PREFIX}{}", self.0)
    }
}

impl FromStr for EventSeq {
    type Err = ValueParseError;

    /// Reads `e:<seq>`.
    ///
    /// # Errors
    /// Returns the form expected when `raw` is not one.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .strip_prefix(EVENT_PREFIX)
            .and_then(|seq| seq.parse().ok())
            .map(Self)
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(EVENT_CURSOR_FORM)
                    .found(raw)
                    .build()
            })
    }
}

/// What happened to a process. Stand-in for package B's `EventKind`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    /// A run began.
    Started,
    /// Its readiness check passed.
    Ready,
    /// Its readiness check timed out.
    ReadyTimeout,
    /// It exited.
    Exited,
    /// It crashed.
    Crashed,
    /// Its policy restarts it after a backoff.
    Restarting,
    /// It was stopped by request.
    Stopped,
    /// Another process holds a port it needs.
    Blocked,
    /// A warning or error notice.
    Notice,
}

/// One lifecycle event. Stand-in for package B's `StackEvent`.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct StackEvent {
    /// Its place in the service's sequence.
    #[getset(get_copy = "pub")]
    seq: EventSeq,
    /// When it happened.
    #[getset(get_copy = "pub")]
    at: SystemTime,
    /// The process, or none for the project as a whole.
    #[getset(get = "pub")]
    process: Option<ProcessName>,
    /// What happened.
    #[getset(get_copy = "pub")]
    kind: EventKind,
    /// A short description, such as `exit 1`.
    #[getset(get = "pub")]
    detail: String,
    /// The process's output cursor when it happened.
    #[getset(get_copy = "pub")]
    cursor: Option<OutputCursor>,
}

/// A page of events. Stand-in for package B's `EventPage`.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct EventPage {
    /// The events, oldest first.
    #[getset(get = "pub")]
    events: Vec<StackEvent>,
    /// Pass as `since` to read only newer events.
    #[getset(get_copy = "pub")]
    cursor: EventSeq,
    /// Whether events after `since` were dropped from the ring.
    #[getset(get_copy = "pub")]
    gap: bool,
}

/// The events of `project` after `since`, of `kinds` (all when empty),
/// waiting at most `wait` for one. Stand-in for package B's
/// `ServiceRequest::Events`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package B lands.
pub fn events(
    _client: &ServiceClient,
    _project: &Project,
    _since: Option<EventSeq>,
    _kinds: &[EventKind],
    _wait: Duration,
) -> Result<EventPage, ToolError> {
    Err(ToolError::Pending(EVENTS))
}
