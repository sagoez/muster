use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{blocked_view::BlockedView, last_exit_view::LastExitView};
use crate::{
    adapter::service::{ProcessStatus, StackUrl},
    domain::{
        port::ListeningSocket,
        process::{ProcessState, Readiness},
        value::{OutputCursor, Pid, ProcessName},
    },
};

/// One process as `stack_status` shows it; a field with no value is left
/// out to save the agent's tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct StackProcessView {
    /// Its name.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Its lifecycle state.
    #[getset(get_copy = "pub")]
    #[serde(with = "crate::adapter::wire::form")]
    state: ProcessState,
    /// Whether its current run passed its readiness check, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    #[serde(with = "crate::adapter::wire::optional_form")]
    ready: Option<Readiness>,
    /// The OS process id while a child runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    pid: Option<Pid>,
    /// Whole seconds the running child has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    uptime_s: Option<u64>,
    /// Times it was restarted.
    #[getset(get_copy = "pub")]
    restarts: u32,
    /// The TCP ports its process tree listens on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[getset(get = "pub")]
    #[serde(with = "crate::adapter::wire::form_list")]
    ports: Vec<ListeningSocket>,
    /// The URLs it serves: declared by its readiness check, else detected.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[getset(get = "pub")]
    urls: Vec<StackUrl>,
    /// Where the current run's output begins; pass it as since to
    /// process_logs to read only this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    run_start: Option<OutputCursor>,
    /// CPU use of its process tree, where 100 is one core.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    cpu_percent: Option<f32>,
    /// Resident memory of its process tree, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    memory_bytes: Option<u64>,
    /// How its last child ended, while it is not running.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    last_exit: Option<LastExitView>,
    /// Why its last start was refused, while it is not running.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    blocked_on: Option<BlockedView>,
}

impl From<&ProcessStatus> for StackProcessView {
    fn from(status: &ProcessStatus) -> Self {
        let usage = status.usage().as_ref();
        Self {
            name: status.name().clone(),
            state: *status.state(),
            ready: Some(*status.ready()).filter(|ready| *ready != Readiness::None),
            pid: *status.pid(),
            uptime_s: status.uptime().map(|uptime| uptime.as_secs()),
            restarts: *status.restarts(),
            ports: status.ports().clone(),
            urls: status.urls().clone(),
            run_start: *status.run_start(),
            cpu_percent: usage.map(|usage| *usage.cpu_percent()),
            memory_bytes: usage.map(|usage| *usage.memory_bytes()),
            last_exit: status.last_exit().as_ref().map(LastExitView::from),
            blocked_on: status.blocked_on().as_ref().map(BlockedView::from),
        }
    }
}
