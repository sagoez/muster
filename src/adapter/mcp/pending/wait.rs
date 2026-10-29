use std::time::Duration;

use getset::{CopyGetters, Getters};
use nutype::nutype;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use typed_builder::TypedBuilder;

use super::output_cursor::OutputCursor;
use crate::{
    adapter::{
        client::ServiceClient,
        mcp::tool_error::ToolError,
        service::{ProcessAction, ProcessStatus},
    },
    domain::{
        process::ProcessState,
        project::Project,
        value::{PositiveDuration, ProcessName},
    },
};

/// Longest pattern a log condition takes.
const PATTERN_MAX_CHARS: usize = 512;
/// Hosts a `port` or `http` condition may probe.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
/// Schemes an `http` condition accepts.
const HTTP_SCHEMES: [&str; 2] = ["http://", "https://"];
/// The feature these stand-ins wait for.
const WAITS: &str = "waiting on a process (package B)";
/// The feature the readiness stand-in waits for.
const READINESS: &str = "stored readiness checks (package B)";

/// Whether `url` names a loopback host over HTTP.
fn is_loopback_url(url: &str) -> bool {
    HTTP_SCHEMES
        .iter()
        .find_map(|scheme| url.strip_prefix(scheme))
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|authority| {
            LOOPBACK_HOSTS.iter().any(|host| {
                authority == *host
                    || authority
                        .strip_prefix(host)
                        .is_some_and(|port| port.starts_with(':'))
            })
        })
}

/// The text of a log condition. Stand-in for package B's `PatternText`.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = PATTERN_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, Serialize, Deserialize)
)]
pub struct PatternText(String);

/// How a log condition matches a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternKind {
    /// A substring of one line.
    Substring,
    /// A regex over one line.
    Regex,
}

/// A log condition. Stand-in for package B's `domain/process/ready.rs`.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct LogPattern {
    /// What to look for.
    #[getset(get = "pub")]
    text: PatternText,
    /// How to match it.
    #[getset(get_copy = "pub")]
    kind: PatternKind,
}

/// A TCP port on localhost. Stand-in for package A's `LocalPort`.
#[nutype(
    validate(greater_or_equal = 1),
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, Serialize, Deserialize)
)]
pub struct LocalPort(u16);

/// An HTTP URL whose host is loopback. Stand-in for package A's `LocalUrl`.
#[nutype(
    sanitize(trim),
    validate(predicate = |url: &str| is_loopback_url(url)),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, Serialize, Deserialize)
)]
pub struct LocalUrl(String);

/// A lifecycle state a wait reaches: one `ps` shows, or `stopped` for not
/// running at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateGoal {
    /// Exactly this state.
    State(ProcessState),
    /// Not running, for any reason.
    Stopped,
}

/// What a wait waits for. Stand-in for package B's `WaitCondition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaitCondition {
    /// A line of output matches.
    Log(LogPattern),
    /// A TCP connect to localhost succeeds.
    Port(LocalPort),
    /// An HTTP response below 500, or with `status`.
    Http {
        /// The URL to ask.
        url: LocalUrl,
        /// The status to wait for.
        status: Option<u16>,
    },
    /// A lifecycle state.
    State(StateGoal),
    /// The stored readiness check passed.
    Ready,
}

/// One wait. Stand-in for package B's `WaitSpec`.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct WaitSpec {
    /// The condition, or the process's default when absent.
    #[getset(get = "pub")]
    until: Option<WaitCondition>,
    /// Output after this counts; without it only output after the call.
    #[getset(get_copy = "pub")]
    since: Option<OutputCursor>,
    /// Longest the wait runs.
    #[getset(get_copy = "pub")]
    timeout: Duration,
}

/// How a wait ended. Stand-in for package B's `WaitStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum WaitStatus {
    /// The condition holds.
    Matched,
    /// The bound passed first.
    Waiting,
    /// The process exited before the condition held.
    Exited,
    /// It crashed.
    Crashed,
    /// It was stopped while waiting.
    Stopped,
    /// It was not running and no start was asked.
    NotRunning,
    /// Its readiness check timed out.
    ReadyTimeout,
}

impl WaitStatus {
    /// Whether the wait ended in a way the agent must act on.
    pub fn is_failure(self) -> bool {
        !matches!(self, Self::Matched | Self::Waiting)
    }
}

/// The line that satisfied a log condition.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct MatchedLine {
    /// The line, stripped of escapes.
    #[getset(get = "pub")]
    line: String,
    /// Where it began.
    #[getset(get_copy = "pub")]
    at: OutputCursor,
}

/// What a wait came to. Stand-in for package B's `WaitOutcome`.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct WaitOutcome {
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// How it ended.
    #[getset(get_copy = "pub")]
    status: WaitStatus,
    /// The condition it waited for.
    #[getset(get = "pub")]
    until: WaitCondition,
    /// The line that matched a log condition.
    #[getset(get = "pub")]
    matched: Option<MatchedLine>,
    /// Where the wait stopped examining output.
    #[getset(get_copy = "pub")]
    cursor: OutputCursor,
    /// How long it waited.
    #[getset(get_copy = "pub")]
    elapsed: Duration,
    /// The run's last lines, for a wait that did not match.
    #[getset(get = "pub")]
    tail: Vec<String>,
    /// Where scanning began, when `since` was older than the service holds.
    #[getset(get_copy = "pub")]
    scanned_from: Option<OutputCursor>,
}

/// Waits on the process `name` of `project`. Stand-in for package B's
/// `ServiceRequest::Wait`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package B lands.
pub fn wait(
    _client: &ServiceClient,
    _project: &Project,
    _name: &ProcessName,
    _spec: &WaitSpec,
) -> Result<WaitOutcome, ToolError> {
    Err(ToolError::Pending(WAITS))
}

/// Applies `action` to `name` and waits on the run it creates, in one
/// request. Stand-in for package B's `ServiceRequest::Control { wait }`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package B lands.
pub fn control_and_wait(
    _client: &ServiceClient,
    _project: &Project,
    _name: &ProcessName,
    _action: ProcessAction,
    _spec: &WaitSpec,
) -> Result<WaitOutcome, ToolError> {
    Err(ToolError::Pending(WAITS))
}

/// Starts the project's autostart processes and waits on each. Stand-in for
/// package B's `ServiceRequest::Up { wait }`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package B lands.
pub fn up_and_wait(
    _client: &ServiceClient,
    _project: &Project,
    _spec: &WaitSpec,
) -> Result<Vec<WaitOutcome>, ToolError> {
    Err(ToolError::Pending(WAITS))
}

/// The run-start cursor `stack_status` reports for `status`. Stand-in for
/// package B's `ProcessStatus::run_start`.
pub fn run_start(_status: &ProcessStatus) -> Option<OutputCursor> {
    None
}

/// What a stored readiness check probes. Stand-in for package B's
/// `ReadyProbe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyProbe {
    /// A line of output matches.
    Log(LogPattern),
    /// A TCP connect to localhost succeeds.
    Port(LocalPort),
    /// An HTTP response below 500.
    Http(LocalUrl),
}

/// A process's stored readiness check. Stand-in for package B's
/// `ReadyCheck`.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct ReadyCheck {
    /// What it probes.
    #[getset(get = "pub")]
    probe: ReadyProbe,
    /// How long a run may take to pass it.
    #[getset(get_copy = "pub")]
    timeout: PositiveDuration,
}

/// Stores `check` as a process's readiness, or clears it. Stand-in for
/// package B's `ready` field on `ProcessDraft` and `ProcessChange`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package B lands.
pub fn store_ready(_check: Option<&ReadyCheck>) -> Result<(), ToolError> {
    Err(ToolError::Pending(READINESS))
}
