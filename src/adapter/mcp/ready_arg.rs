use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::{
    condition_arg::{local_port, local_url, log_pattern},
    pending::{ReadyCheck, ReadyProbe},
    process_fields::invalid,
    tool_error::ToolError,
};
use crate::domain::value::PositiveDuration;

/// The `ready` field of `process_add` and `process_set`.
const READY_FIELD: &str = "ready";
/// What a readiness check must name exactly one of.
const ONE_PROBE_FORM: &str = "exactly one of log, port or http, and a timeout";
/// What `regex` needs beside it.
const REGEX_FORM: &str = "regex only together with log";
/// The form a timeout is written in.
const TIMEOUT_FORM: &str = "a positive duration such as 30s";
/// Separates the probes a refused check named.
const PROBE_SEPARATOR: &str = ", ";
/// How a check naming no probe is described.
const NO_PROBE: &str = "none";

/// A stored readiness check as an agent writes it, such as
/// {"log": "Listening on", "timeout": "60s"}.
#[derive(
    Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder,
)]
#[serde(deny_unknown_fields)]
pub struct ReadyArg {
    /// Ready once a line of output contains this text; a regex with
    /// regex=true.
    #[serde(default)]
    #[builder(default)]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Read log as a regex over one line.
    #[serde(default)]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// Ready once this localhost TCP port accepts a connection.
    #[serde(default)]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    port: Option<u16>,
    /// Ready once this localhost URL answers below 500.
    #[serde(default)]
    #[builder(default)]
    #[getset(get = "pub")]
    http: Option<String>,
    /// How long a run may take to become ready, such as 60s.
    #[getset(get = "pub")]
    timeout: String,
}

impl ReadyArg {
    /// The readiness check these fields describe.
    ///
    /// # Errors
    /// Returns the refusal naming what is wrong: no probe or more than one,
    /// a regex without a log, a value out of form or a timeout that is not
    /// a positive duration.
    pub fn check(&self) -> Result<ReadyCheck, ToolError> {
        let named: Vec<&str> = [
            ("log", self.log.is_some()),
            ("port", self.port.is_some()),
            ("http", self.http.is_some()),
        ]
        .into_iter()
        .filter_map(|(probe, set)| set.then_some(probe))
        .collect();
        if named.len() != 1 {
            let found = if named.is_empty() {
                NO_PROBE.to_string()
            } else {
                named.join(PROBE_SEPARATOR)
            };
            return Err(invalid(READY_FIELD, ONE_PROBE_FORM, &found));
        }
        if let Some(regex) = self.regex.filter(|_| self.log.is_none()) {
            return Err(invalid(READY_FIELD, REGEX_FORM, &regex.to_string()));
        }
        let probe = match (&self.log, self.port, &self.http) {
            (Some(text), ..) => {
                ReadyProbe::Log(log_pattern(READY_FIELD, text, self.regex.unwrap_or(false))?)
            },
            (_, Some(port), _) => ReadyProbe::Port(local_port(READY_FIELD, port)?),
            (_, _, Some(url)) => ReadyProbe::Http(local_url(READY_FIELD, url)?),
            (None, None, None) => return Err(invalid(READY_FIELD, ONE_PROBE_FORM, NO_PROBE)),
        };
        let timeout = self
            .timeout
            .parse::<PositiveDuration>()
            .map_err(|_| invalid(READY_FIELD, TIMEOUT_FORM, &self.timeout))?;
        Ok(ReadyCheck::builder().probe(probe).timeout(timeout).build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check an agent sent as `json`.
    fn read(json: serde_json::Value) -> Result<ReadyCheck, ToolError> {
        serde_json::from_value::<ReadyArg>(json).unwrap().check()
    }

    /// Each probe reads with its timeout.
    #[test]
    fn each_probe_reads() {
        assert!(matches!(
            read(serde_json::json!({ "log": "Listening on", "timeout": "60s" }))
                .unwrap()
                .probe(),
            ReadyProbe::Log(_)
        ));
        assert!(matches!(
            read(serde_json::json!({ "port": 3000, "timeout": "30s" }))
                .unwrap()
                .probe(),
            ReadyProbe::Port(_)
        ));
        assert!(matches!(
            read(serde_json::json!({ "http": "http://127.0.0.1:3000/health", "timeout": "30s" }))
                .unwrap()
                .probe(),
            ReadyProbe::Http(_)
        ));
    }

    /// Two probes, none, a bad timeout or a non-loopback URL are refused
    /// under the ready field; a missing timeout fails to read at all.
    #[test]
    fn a_check_out_of_form_is_refused() {
        for json in [
            serde_json::json!({ "log": "x", "port": 1, "timeout": "1s" }),
            serde_json::json!({ "timeout": "1s" }),
            serde_json::json!({ "port": 1, "timeout": "0s" }),
            serde_json::json!({ "http": "http://10.0.0.1/", "timeout": "1s" }),
        ] {
            let refused = read(json).unwrap_err().to_string();
            assert!(refused.contains("invalid ready"), "{refused}");
        }
        assert!(serde_json::from_value::<ReadyArg>(serde_json::json!({ "port": 1 })).is_err());
    }
}
