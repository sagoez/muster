use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::exit_view::ExitView;
use crate::{
    adapter::service::ProcessStatus,
    domain::{
        process::{ActivityState, ProcessKind, ProcessState},
        value::ProcessName,
    },
};

/// One process as `--json` shows it. Durations are whole seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProcessView {
    /// Its name within its project.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Its section of `muster.yml`.
    #[getset(get_copy = "pub")]
    kind: ProcessKind,
    /// Its lifecycle state.
    #[getset(get_copy = "pub")]
    state: ProcessState,
    /// Whether it is working, idle or asking for attention.
    #[getset(get_copy = "pub")]
    activity: ActivityState,
    /// The OS process id while a child runs.
    #[getset(get_copy = "pub")]
    pid: Option<u32>,
    /// Whole seconds the running child has run.
    #[getset(get_copy = "pub")]
    uptime: Option<u64>,
    /// Times it was restarted, by its policy or on request.
    #[getset(get_copy = "pub")]
    restarts: u32,
    /// CPU use of its process tree, as a percentage where 100 is one core.
    #[getset(get_copy = "pub")]
    cpu_percent: Option<f32>,
    /// Resident memory of its process tree, in bytes.
    #[getset(get_copy = "pub")]
    memory_bytes: Option<u64>,
    /// Writable clients attached to its terminal.
    #[getset(get_copy = "pub")]
    attached: u32,
    /// How its last child ended, while it is not running.
    #[getset(get = "pub")]
    last_exit: Option<ExitView>,
    /// Whole seconds a graceful stop waits before it kills, or none when a
    /// stop kills at once.
    #[getset(get_copy = "pub")]
    stop_grace: Option<u64>,
}

impl From<&ProcessStatus> for ProcessView {
    fn from(status: &ProcessStatus) -> Self {
        let usage = status.usage().as_ref();
        Self {
            name: status.name().clone(),
            kind: *status.kind(),
            state: *status.state(),
            activity: *status.activity(),
            pid: *status.pid(),
            uptime: status.uptime().map(|uptime| uptime.as_secs()),
            restarts: *status.restarts(),
            cpu_percent: usage.map(|usage| *usage.cpu_percent()),
            memory_bytes: usage.map(|usage| *usage.memory_bytes()),
            attached: *status.attached(),
            last_exit: status.last_exit().as_ref().map(ExitView::from),
            stop_grace: status.stop_grace().map(|grace| grace.as_secs()),
        }
    }
}
