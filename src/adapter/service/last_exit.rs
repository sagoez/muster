use std::time::SystemTime;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::exit_reason::ExitReason;
use crate::domain::pty::ExitOutcome;

/// How a process's most recent run ended.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct LastExit {
    /// Its exit status or signal.
    #[getset(get = "pub")]
    #[serde(with = "crate::adapter::wire::form")]
    outcome: ExitOutcome,
    /// Why it ended.
    #[getset(get_copy = "pub")]
    reason: ExitReason,
    /// When it ended, when this service saw it end.
    #[getset(get = "pub")]
    #[serde(with = "humantime_serde")]
    #[builder(default)]
    at: Option<SystemTime>,
}
