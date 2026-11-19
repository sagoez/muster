use getset::{CopyGetters, Getters};
use muster_domain::{
    process::{ReadyCheck, ReadyProbe},
    text::{ListForm, ValueList},
    value::PositiveDuration,
};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use crate::{
    process::{
        definition::{process_fields::invalid, ready_field::ReadyField, seconds_arg::SecondsArg},
        lifecycle::condition_arg::{local_port, local_url, log_pattern},
    },
    result::tool_error::ToolError,
    runtime::arg_field::ArgField,
};

/// What a readiness check must name beside its one probe.
const AND_A_TIMEOUT: &str = ", and a timeout";
/// What `regex` needs beside it.
const REGEX_FORM: &str = "regex only together with log";
/// The form a timeout is written in.
const TIMEOUT_FORM: &str = "a positive number of seconds in whole milliseconds, such as 30 or 1.5";

/// A stored readiness check as an agent writes it, such as
/// {"log": "Listening on", "timeout": 60}.
#[derive(
    Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder,
)]
#[serde(deny_unknown_fields)]
pub struct ReadyArg {
    /// Ready once a line of output contains this text; a regex with
    /// regex=true.
    #[builder(default)]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Read log as a regex over one line.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// Ready once this localhost TCP port accepts a connection.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    port: Option<u16>,
    /// Ready once this localhost URL answers below 500.
    #[builder(default)]
    #[getset(get = "pub")]
    http: Option<String>,
    /// Seconds a run may take to become ready, such as 60.
    #[getset(get = "pub")]
    timeout: SecondsArg,
}

impl ReadyArg {
    /// The readiness check these fields describe.
    ///
    /// # Errors
    /// Returns the refusal naming what is wrong: no probe or more than one,
    /// a regex without a log, a value out of form or a timeout that is not
    /// a positive duration.
    pub fn check(&self) -> Result<ReadyCheck, ToolError> {
        let named: Vec<ReadyField> = ReadyField::probes()
            .filter(|probe| self.gives(*probe))
            .collect();
        if named.len() != 1 {
            return Err(Self::not_one_probe(named));
        }
        if let Some(regex) = self.regex.filter(|_| self.log.is_none()) {
            return Err(invalid(ArgField::Ready, REGEX_FORM, &regex.to_string()));
        }
        let probe = match (&self.log, self.port, &self.http) {
            (Some(text), ..) => ReadyProbe::Log(log_pattern(
                ArgField::Ready,
                text,
                self.regex.unwrap_or(false),
            )?),
            (_, Some(port), _) => ReadyProbe::Port(local_port(ArgField::Ready, port)?),
            (_, _, Some(url)) => ReadyProbe::Http(local_url(ArgField::Ready, url)?),
            (None, None, None) => return Err(Self::not_one_probe(Vec::new())),
        };
        let timeout = self
            .timeout
            .duration()
            .and_then(|timeout| PositiveDuration::try_new(timeout).ok())
            .ok_or_else(|| invalid(ArgField::Ready, TIMEOUT_FORM, &self.timeout.written()))?;
        Ok(ReadyCheck::builder().probe(probe).timeout(timeout).build())
    }

    /// Whether these fields give a value for `field`; the timeout is always
    /// given.
    fn gives(&self, field: ReadyField) -> bool {
        match field {
            ReadyField::Log => self.log.is_some(),
            ReadyField::Regex => self.regex.is_some(),
            ReadyField::Port => self.port.is_some(),
            ReadyField::Http => self.http.is_some(),
            ReadyField::Timeout => true,
        }
    }

    /// The refusal of a check that names the probes `named`, not exactly one.
    fn not_one_probe(named: Vec<ReadyField>) -> ToolError {
        invalid(
            ArgField::Ready,
            &format!(
                "{}{AND_A_TIMEOUT}",
                ValueList::of_values(ReadyField::probes()).written(ListForm::ExactlyOne)
            ),
            &ValueList::of_values(named).written_or_none(ListForm::Commas),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Duration};

    use serde_json::Value;
    use strum::IntoEnumIterator;

    use super::*;
    use crate::runtime::test_support::arguments_of;

    /// The fields [`ReadyField`] names are exactly the arguments a check
    /// takes, so a new argument is a probe or goes with one.
    #[test]
    fn the_ready_fields_are_its_arguments() {
        let fields: BTreeSet<String> = ReadyField::iter().map(|field| field.to_string()).collect();
        assert_eq!(fields, arguments_of::<ReadyArg>());
    }

    /// The check an agent sent as `json`.
    fn read(json: Value) -> Result<ReadyCheck, ToolError> {
        serde_json::from_value::<ReadyArg>(json).unwrap().check()
    }

    /// Each probe reads with its timeout.
    #[test]
    fn each_probe_reads() {
        assert!(matches!(
            read(serde_json::json!({ "log": "Listening on", "timeout": 60 }))
                .unwrap()
                .probe(),
            ReadyProbe::Log(_)
        ));
        assert!(matches!(
            read(serde_json::json!({ "port": 3000, "timeout": 30 }))
                .unwrap()
                .probe(),
            ReadyProbe::Port(_)
        ));
        let http =
            read(serde_json::json!({ "http": "http://127.0.0.1:3000/health", "timeout": 2.5 }))
                .unwrap();
        assert!(matches!(http.probe(), ReadyProbe::Http(_)));
        assert_eq!(http.timeout().into_inner(), Duration::from_millis(2500));
    }

    /// Two probes, none, a bad timeout or a non-loopback URL are refused
    /// under the ready field; a missing timeout fails to read at all.
    #[test]
    fn a_check_out_of_form_is_refused() {
        for json in [
            serde_json::json!({ "log": "x", "port": 1, "timeout": 1 }),
            serde_json::json!({ "timeout": 1 }),
            serde_json::json!({ "port": 1, "timeout": 0 }),
            serde_json::json!({ "port": 1, "timeout": -5 }),
            serde_json::json!({ "http": "http://10.0.0.1/", "timeout": 1 }),
        ] {
            let refused = read(json).unwrap_err().to_string();
            assert!(refused.contains("invalid ready"), "{refused}");
        }
        assert!(serde_json::from_value::<ReadyArg>(serde_json::json!({ "port": 1 })).is_err());
    }

    /// A check naming no probe or two lists the probes it takes, read from
    /// their enum in the words agents have always seen.
    #[test]
    fn a_refusal_lists_the_probes_in_full() {
        let expected = "invalid ready: expected exactly one of log, port or http, and a timeout";
        assert_eq!(
            read(serde_json::json!({ "timeout": 1 }))
                .unwrap_err()
                .to_string(),
            format!("{expected}, got 'none'")
        );
        assert_eq!(
            read(serde_json::json!({ "log": "x", "http": "http://localhost/", "timeout": 1 }))
                .unwrap_err()
                .to_string(),
            format!("{expected}, got 'log, http'")
        );
    }

    /// A timeout finer than a millisecond is refused naming the rule, so the
    /// stored timeout always reads back as it was sent.
    #[test]
    fn a_timeout_is_whole_milliseconds() {
        for fine in [0.0004, 1.0007] {
            let refused = read(serde_json::json!({ "port": 1, "timeout": fine }))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("whole milliseconds"), "{refused}");
        }
    }

    /// A timeout written as humantime text is refused naming seconds, the
    /// form every result reports a timeout in.
    #[test]
    fn a_timeout_as_text_is_refused_naming_seconds() {
        let refused =
            serde_json::from_value::<ReadyArg>(serde_json::json!({ "port": 1, "timeout": "30s" }))
                .unwrap_err()
                .to_string();

        assert!(refused.contains("number of seconds"), "{refused}");
    }
}
