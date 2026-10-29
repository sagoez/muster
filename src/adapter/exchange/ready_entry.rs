use std::time::Duration;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::EntryError;
use crate::{
    adapter::holder::LineMatcher,
    domain::{
        process::{PatternKind, ReadyCheck, ReadyProbe},
        value::{LocalPort, LocalUrl, PositiveDuration},
    },
};

/// Why a check naming no probe, or more than one, is refused.
const ONE_PROBE: &str = "name exactly one of log, port or http";
/// Why `regex` without `log` is refused.
const REGEX_WITHOUT_LOG: &str = "regex goes only with log";
/// Why port zero is refused.
const ZERO_PORT: &str = "port must be from 1 to 65535";
/// Why a zero timeout is refused.
const ZERO_TIMEOUT: &str = "timeout must be longer than zero";

/// An exported process's readiness check, as the stack file writes it:
/// exactly one of `log`, `port` or `http`, and a `timeout`.
#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(deny_unknown_fields)]
pub struct ReadyEntry {
    /// Ready once an output line contains this text, or matches it with
    /// `regex`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Whether `log` is a regex.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// Ready once this local TCP port accepts a connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    port: Option<u16>,
    /// Ready once this local URL answers below 500.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    http: Option<String>,
    /// How long a run may take to pass it, in humantime (`60s`).
    #[serde(with = "humantime_serde")]
    #[getset(get_copy = "pub")]
    timeout: Duration,
}

impl From<&ReadyCheck> for ReadyEntry {
    fn from(check: &ReadyCheck) -> Self {
        let timeout = check.timeout().into_inner();
        match check.probe() {
            ReadyProbe::Log(pattern) => Self {
                log: Some(pattern.text().to_string()),
                regex: (*pattern.kind() == PatternKind::Regex).then_some(true),
                port: None,
                http: None,
                timeout,
            },
            ReadyProbe::Port(port) => Self {
                log: None,
                regex: None,
                port: Some(port.into_inner()),
                http: None,
                timeout,
            },
            ReadyProbe::Http(url) => Self {
                log: None,
                regex: None,
                port: None,
                http: Some(url.to_string()),
                timeout,
            },
        }
    }
}

impl TryFrom<ReadyEntry> for ReadyCheck {
    type Error = EntryError;

    /// The check `entry` describes, its pattern compiled before it is
    /// stored.
    ///
    /// # Errors
    /// Returns [`EntryError::InvalidReady`] naming what is wrong.
    fn try_from(entry: ReadyEntry) -> Result<Self, Self::Error> {
        let invalid = |reason: String| EntryError::InvalidReady { reason };
        if entry.regex.is_some() && entry.log.is_none() {
            return Err(invalid(REGEX_WITHOUT_LOG.to_string()));
        }
        let probe = match (entry.log, entry.port, entry.http) {
            (Some(text), None, None) => ReadyProbe::Log(
                LineMatcher::checked(&text, PatternKind::of_regex(entry.regex.unwrap_or(false)))
                    .map_err(|error| invalid(error.to_string()))?,
            ),
            (None, Some(port), None) => ReadyProbe::Port(
                LocalPort::try_new(port).map_err(|_| invalid(ZERO_PORT.to_string()))?,
            ),
            (None, None, Some(url)) => ReadyProbe::Http(
                url.parse::<LocalUrl>()
                    .map_err(|error| invalid(error.to_string()))?,
            ),
            _ => return Err(invalid(ONE_PROBE.to_string())),
        };
        let timeout = PositiveDuration::try_new(entry.timeout)
            .map_err(|_| invalid(ZERO_TIMEOUT.to_string()))?;
        Ok(ReadyCheck::builder().probe(probe).timeout(timeout).build())
    }
}
