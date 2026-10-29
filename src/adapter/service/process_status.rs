use std::time::Duration;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::process_usage::ProcessUsage;
use crate::domain::{
    process::{ActivityState, ProcessKind, ProcessState},
    pty::ExitOutcome,
    value::ProcessName,
};

/// What `ps` shows for one supervised process.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProcessStatus {
    /// The process's name within its project.
    name: ProcessName,
    /// Section the process belongs to.
    kind: ProcessKind,
    /// Lifecycle state.
    state: ProcessState,
    /// Inferred working, idle or attention state.
    activity: ActivityState,
    /// OS process id while a child is live.
    pid: Option<u32>,
    /// How long the live child has run.
    #[serde(with = "humantime_serde")]
    uptime: Option<Duration>,
    /// Times the process was restarted, by its policy or on request.
    restarts: u32,
    /// Latest sampled resource usage.
    usage: Option<ProcessUsage>,
    /// Writable clients attached to its terminal.
    attached: u32,
    /// How its most recent child ended, while it is not running.
    last_exit: Option<ExitOutcome>,
    /// How long a graceful stop waits before it kills, or none when a stop
    /// kills at once.
    #[serde(with = "humantime_serde")]
    stop_grace: Option<Duration>,
}
