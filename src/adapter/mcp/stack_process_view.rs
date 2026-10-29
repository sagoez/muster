use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::pending::run_start;
use crate::{
    adapter::{client::ExitView, service::ProcessStatus},
    domain::{process::ProcessState, value::ProcessName},
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
    state: ProcessState,
    /// The OS process id while a child runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    pid: Option<u32>,
    /// Whole seconds the running child has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    uptime_s: Option<u64>,
    /// Times it was restarted.
    #[getset(get_copy = "pub")]
    restarts: u32,
    /// Where the current run's output begins; pass it as since to
    /// process_logs to read only this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    run_start: Option<String>,
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
    last_exit: Option<ExitView>,
}

impl From<&ProcessStatus> for StackProcessView {
    fn from(status: &ProcessStatus) -> Self {
        let usage = status.usage().as_ref();
        Self {
            name: status.name().clone(),
            state: *status.state(),
            pid: *status.pid(),
            uptime_s: status.uptime().map(|uptime| uptime.as_secs()),
            restarts: *status.restarts(),
            run_start: run_start(status).map(|cursor| cursor.to_string()),
            cpu_percent: usage.map(|usage| *usage.cpu_percent()),
            memory_bytes: usage.map(|usage| *usage.memory_bytes()),
            last_exit: status
                .last_exit()
                .as_ref()
                .map(|exit| ExitView::from(exit.outcome())),
        }
    }
}
