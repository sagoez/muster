use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{
    adapter::service::WaitCondition,
    domain::{
        process::{LogPattern, PatternKind, ReadyProbe},
        value::{LocalPort, LocalUrl},
    },
};

/// A wait condition in the shape an agent writes one: exactly one of `log`,
/// `port`, `http`, `state` or `ready`, with `regex` or `status` beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ConditionView {
    /// A line of output containing this text, or matching it with `regex`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Whether `log` is a regex.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// A local TCP port that accepts a connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    port: Option<u16>,
    /// A local URL that answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    http: Option<String>,
    /// The HTTP status waited for, with `http`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    status: Option<u16>,
    /// A lifecycle state.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    state: Option<String>,
    /// The stored readiness check passing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    ready: Option<bool>,
}

impl ConditionView {
    /// A condition on lines matching `pattern`.
    fn for_log(pattern: &LogPattern) -> Self {
        Self {
            log: Some(pattern.text().to_string()),
            regex: (*pattern.kind() == PatternKind::Regex).then_some(true),
            ..Self::default()
        }
    }

    /// A condition on `port` accepting a connection.
    fn for_port(port: LocalPort) -> Self {
        Self {
            port: Some(port.into_inner()),
            ..Self::default()
        }
    }

    /// A condition on `url` answering with `status`, or below 500.
    fn for_http(url: &LocalUrl, status: Option<u16>) -> Self {
        Self {
            http: Some(url.to_string()),
            status,
            ..Self::default()
        }
    }
}

impl From<&ReadyProbe> for ConditionView {
    fn from(probe: &ReadyProbe) -> Self {
        match probe {
            ReadyProbe::Log(pattern) => Self::for_log(pattern),
            ReadyProbe::Port(port) => Self::for_port(*port),
            ReadyProbe::Http(url) => Self::for_http(url, None),
        }
    }
}

impl From<&WaitCondition> for ConditionView {
    fn from(condition: &WaitCondition) -> Self {
        match condition {
            WaitCondition::Log(pattern) => Self::for_log(pattern),
            WaitCondition::Port(port) => Self::for_port(*port),
            WaitCondition::Http { url, status } => Self::for_http(url, *status),
            WaitCondition::State(goal) => Self {
                state: Some(goal.to_string()),
                ..Self::default()
            },
            WaitCondition::Ready => Self {
                ready: Some(true),
                ..Self::default()
            },
        }
    }
}
