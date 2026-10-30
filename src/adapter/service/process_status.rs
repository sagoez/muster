use std::time::Duration;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{
    last_exit::LastExit, port_block::PortBlock, process_usage::ProcessUsage, stack_url::StackUrl,
    state_goal::QUICK_EXIT_WINDOW,
};
use crate::domain::{
    port::ListeningSocket,
    process::{ProcessState, Readiness},
    value::{OutputCursor, Pid, ProcessName},
};

/// What `ps` shows for one supervised process.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProcessStatus {
    /// The process's name within its project.
    name: ProcessName,
    /// Lifecycle state.
    #[serde(with = "crate::adapter::wire::form")]
    state: ProcessState,
    /// Whether its current run passed its readiness check.
    #[builder(default)]
    #[serde(with = "crate::adapter::wire::form")]
    ready: Readiness,
    /// OS process id while a child is live.
    pid: Option<Pid>,
    /// How long the live child has run.
    #[serde(with = "humantime_serde")]
    uptime: Option<Duration>,
    /// Times the process was restarted, by its policy or on request.
    restarts: u32,
    /// Latest sampled resource usage.
    usage: Option<ProcessUsage>,
    /// Writable clients attached to its terminal.
    attached: u32,
    /// The TCP ports its process tree was last seen listening on.
    #[builder(default)]
    #[serde(with = "crate::adapter::wire::form_list")]
    ports: Vec<ListeningSocket>,
    /// The URLs it serves: the one its readiness check declares, else one per
    /// detected port.
    #[builder(default)]
    urls: Vec<StackUrl>,
    /// Where its current run began in its output.
    #[builder(default)]
    run_start: Option<OutputCursor>,
    /// How its most recent child ended, while it is not running.
    last_exit: Option<LastExit>,
    /// How long a graceful stop waits before it kills.
    #[serde(with = "humantime_serde")]
    stop_grace: Duration,
    /// Why its last start was refused, while it is not running: another
    /// process holds a port it needs.
    #[builder(default)]
    blocked_on: Option<PortBlock>,
}

impl ProcessStatus {
    /// Whether it runs a child that started less than [`QUICK_EXIT_WINDOW`]
    /// ago, which may yet exit at once.
    pub fn in_quick_exit_window(&self) -> bool {
        self.state == ProcessState::Running
            && self.uptime.is_some_and(|uptime| uptime < QUICK_EXIT_WINDOW)
    }
}
