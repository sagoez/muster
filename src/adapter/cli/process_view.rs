use std::time::Duration;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::exit_view::ExitView;
use crate::{
    adapter::service::ProcessStatus,
    domain::{process::ProcessState, value::ProcessName},
};

/// One process as `--json` shows it. Uptime is whole seconds; the stop grace
/// is whole seconds rounded up, so a grace under a second never reads as 0.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProcessView {
    /// Its name within its project.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Its lifecycle state.
    #[getset(get_copy = "pub")]
    state: ProcessState,
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
    /// Seconds, rounded up, a graceful stop waits before it kills, or none
    /// when a stop kills at once.
    #[getset(get_copy = "pub")]
    stop_grace: Option<u64>,
}

impl From<&ProcessStatus> for ProcessView {
    fn from(status: &ProcessStatus) -> Self {
        let usage = status.usage().as_ref();
        Self {
            name: status.name().clone(),
            state: *status.state(),
            pid: *status.pid(),
            uptime: status.uptime().map(|uptime| uptime.as_secs()),
            restarts: *status.restarts(),
            cpu_percent: usage.map(|usage| *usage.cpu_percent()),
            memory_bytes: usage.map(|usage| *usage.memory_bytes()),
            attached: *status.attached(),
            last_exit: status.last_exit().as_ref().map(ExitView::from),
            stop_grace: status.stop_grace().map(whole_seconds_up),
        }
    }
}

/// `duration` in whole seconds, any part of a second counting as one.
fn whole_seconds_up(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grace under a second, or with a fraction, is never shown shorter
    /// than it is.
    #[test]
    fn a_stop_grace_rounds_up() {
        assert_eq!(whole_seconds_up(Duration::from_millis(500)), 1);
        assert_eq!(whole_seconds_up(Duration::from_millis(5_001)), 6);
        assert_eq!(whole_seconds_up(Duration::from_secs(5)), 5);
    }
}
