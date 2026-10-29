use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{process_fields::invalid, tool_error::ToolError};
use crate::{
    adapter::{
        holder::LineMatcher,
        service::{StateGoal, WaitCondition},
    },
    domain::{
        process::{LogPattern, PatternKind},
        value::{LocalPort, LocalUrl},
    },
};

/// What a condition must name exactly one of.
const ONE_CONDITION_FORM: &str = "exactly one of log, port, http, state or ready";
/// What `regex` needs beside it.
const REGEX_FORM: &str = "regex only together with log";
/// What `status` needs beside it.
const STATUS_FORM: &str = "status only together with http";
/// The form a port is written in.
pub(super) const PORT_FORM: &str = "a port from 1 to 65535";
/// The form a URL is written in.
const URL_FORM: &str = "an http URL on localhost, 127.0.0.1 or [::1]";
/// The states a state condition takes.
const STATE_FORM: &str = "running, paused, exited, crashed, restarting or stopped";
/// The only value `ready` takes.
const READY_FORM: &str = "true";
/// The names of a condition's kinds, as an agent writes them.
const LOG: &str = "log";
const PORT: &str = "port";
const HTTP: &str = "http";
const STATE: &str = "state";
const READY: &str = "ready";
/// Separates the kinds a refused condition named.
const KIND_SEPARATOR: &str = ", ";
/// How a condition naming no kind is described.
const NO_KIND: &str = "none";

/// A wait condition as an agent writes it: exactly one of `log`, `port`,
/// `http`, `state` or `ready`.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    Getters,
    CopyGetters,
    TypedBuilder,
)]
#[serde(deny_unknown_fields)]
pub struct ConditionArg {
    /// A line of output containing this text; a regex with regex=true.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Read log as a regex over one line.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// A TCP port on localhost that accepts a connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    port: Option<u16>,
    /// A URL on localhost that answers below 500 (or with status).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    http: Option<String>,
    /// The HTTP status to wait for, with http.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    status: Option<u16>,
    /// A lifecycle state as stack_status shows it; stopped means not running
    /// for any reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    state: Option<String>,
    /// true waits until the process's stored readiness check passes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    ready: Option<bool>,
}

impl ConditionArg {
    /// The condition these fields describe, refused as the field `field`.
    ///
    /// # Errors
    /// Returns [`ToolError::InvalidField`] naming what is wrong: no kind or
    /// more than one, a modifier without its kind, or a value out of form.
    pub fn condition(&self, field: &'static str) -> Result<WaitCondition, ToolError> {
        let named: Vec<&str> = [
            (LOG, self.log.is_some()),
            (PORT, self.port.is_some()),
            (HTTP, self.http.is_some()),
            (STATE, self.state.is_some()),
            (READY, self.ready.is_some()),
        ]
        .into_iter()
        .filter_map(|(kind, set)| set.then_some(kind))
        .collect();
        if named.len() != 1 {
            let found = if named.is_empty() {
                NO_KIND.to_string()
            } else {
                named.join(KIND_SEPARATOR)
            };
            return Err(invalid(field, ONE_CONDITION_FORM, &found));
        }
        if let Some(regex) = self.regex.filter(|_| self.log.is_none()) {
            return Err(invalid(field, REGEX_FORM, &regex.to_string()));
        }
        if let Some(status) = self.status.filter(|_| self.http.is_none()) {
            return Err(invalid(field, STATUS_FORM, &status.to_string()));
        }
        if let Some(text) = &self.log {
            return log_pattern(field, text, self.regex.unwrap_or(false)).map(WaitCondition::Log);
        }
        if let Some(port) = self.port {
            return local_port(field, port).map(WaitCondition::Port);
        }
        if let Some(url) = &self.http {
            return Ok(WaitCondition::Http {
                url: local_url(field, url)?,
                status: self.status,
            });
        }
        if let Some(state) = &self.state {
            return state_goal(field, state).map(WaitCondition::State);
        }
        match self.ready {
            Some(true) => Ok(WaitCondition::Ready),
            other => Err(invalid(
                field,
                READY_FORM,
                &other.map(|ready| ready.to_string()).unwrap_or_default(),
            )),
        }
    }
}

/// The log pattern `text`, a regex when `regex`, checked by compiling it.
///
/// # Errors
/// Returns the refusal of a blank, overlong or uncompilable pattern.
pub fn log_pattern(field: &'static str, text: &str, regex: bool) -> Result<LogPattern, ToolError> {
    LineMatcher::checked(text, PatternKind::of_regex(regex))
        .map_err(|source| ToolError::InvalidPattern { field, source })
}

/// The localhost port `port`.
///
/// # Errors
/// Returns the refusal of port 0.
pub fn local_port(field: &'static str, port: u16) -> Result<LocalPort, ToolError> {
    LocalPort::try_new(port).map_err(|_| invalid(field, PORT_FORM, &port.to_string()))
}

/// The loopback URL `url`.
///
/// # Errors
/// Returns the refusal of a URL whose host is not loopback.
pub fn local_url(field: &'static str, url: &str) -> Result<LocalUrl, ToolError> {
    url.parse().map_err(|_| invalid(field, URL_FORM, url))
}

/// The state goal `raw`.
///
/// # Errors
/// Returns the refusal of anything else, listing the states.
fn state_goal(field: &'static str, raw: &str) -> Result<StateGoal, ToolError> {
    raw.trim()
        .parse()
        .map_err(|_| invalid(field, STATE_FORM, raw))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::adapter::client::ConditionView;

    /// The field refusals are reported under.
    const FIELD: &str = "until";

    /// The condition an agent sent as `json`.
    fn read(json: Value) -> Result<WaitCondition, ToolError> {
        serde_json::from_value::<ConditionArg>(json)
            .unwrap()
            .condition(FIELD)
    }

    /// The refusal text of the condition an agent sent as `json`.
    fn refused(json: Value) -> String {
        read(json).unwrap_err().to_string()
    }

    /// Each kind reads into its condition and echoes back as written.
    #[test]
    fn each_kind_reads_and_echoes() {
        for json in [
            serde_json::json!({ "log": "Listening on" }),
            serde_json::json!({ "log": "ready in \\d+ms", "regex": true }),
            serde_json::json!({ "port": 3000 }),
            serde_json::json!({ "http": "http://localhost:3000/health", "status": 200 }),
            serde_json::json!({ "state": "running" }),
            serde_json::json!({ "state": "stopped" }),
            serde_json::json!({ "ready": true }),
        ] {
            let condition = read(json.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(ConditionView::from(&condition)).unwrap(),
                json
            );
        }
    }

    /// No kind, two kinds, or a modifier without its kind is refused,
    /// naming the field and what it takes.
    #[test]
    fn exactly_one_kind_is_required() {
        let none = refused(serde_json::json!({}));
        assert!(
            none.contains(FIELD) && none.contains("exactly one"),
            "{none}"
        );
        let two = refused(serde_json::json!({ "log": "x", "port": 1 }));
        assert!(two.contains("log, port"), "{two}");
        assert!(refused(serde_json::json!({ "port": 1, "regex": true })).contains("regex only"));
        assert!(refused(serde_json::json!({ "port": 1, "status": 200 })).contains("status only"));
        assert!(refused(serde_json::json!({ "ready": false })).contains("true"));
    }

    /// A host other than loopback, port 0, an unknown state and a blank or
    /// overlong pattern are refused before anything waits.
    #[test]
    fn values_out_of_form_are_refused() {
        assert!(
            refused(serde_json::json!({ "http": "http://example.com/" })).contains("localhost")
        );
        assert!(
            refused(serde_json::json!({ "http": "http://localhost.evil.com/" }))
                .contains("localhost")
        );
        assert!(refused(serde_json::json!({ "port": 0 })).contains("1 to 65535"));
        assert!(refused(serde_json::json!({ "state": "sleeping" })).contains("restarting"));
        assert!(refused(serde_json::json!({ "log": "(", "regex": true })).contains("regex"));
        assert!(refused(serde_json::json!({ "log": "   " })).contains("1 to 512"));
        assert!(refused(serde_json::json!({ "log": "x".repeat(513) })).contains("1 to 512"));
    }

    /// An unknown field is refused when the arguments are read.
    #[test]
    fn an_unknown_field_is_refused() {
        assert!(
            serde_json::from_value::<ConditionArg>(serde_json::json!({ "line": "x" })).is_err()
        );
    }
}
