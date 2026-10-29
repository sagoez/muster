use std::time::Duration;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::wait_condition::WaitCondition;
use crate::domain::value::OutputCursor;

/// One wait as a client asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct WaitSpec {
    /// What to wait for; the process's default when absent.
    #[builder(default)]
    until: Option<WaitCondition>,
    /// Output after this cursor counts; without it, only output after the
    /// request does.
    #[builder(default)]
    since: Option<OutputCursor>,
    /// Longest to wait before answering that it is still waiting.
    #[serde(with = "humantime_serde")]
    timeout: Duration,
}
