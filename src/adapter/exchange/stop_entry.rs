use std::time::Duration;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::process::StopSignal;

/// How an exported process is stopped, as the stack file writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct StopEntry {
    /// The graceful signal.
    signal: StopSignal,
    /// Time allowed before a force-kill, in humantime (`5s`).
    #[serde(with = "humantime_serde")]
    grace_period: Duration,
}
