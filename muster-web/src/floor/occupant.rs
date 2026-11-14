use std::time::Instant;

use muster_client::listening_ports;
use muster_domain::{
    process::{ProcessIcon, ProcessState},
    value::ProcessName,
};
use muster_protocol::service::ProcessStatus;

use super::{
    reading::{LogStat, StackedProcess},
    view::{EndedView, FaceView, Placement},
};

/// One process on the floor and what the floor keeps of it between readings.
pub(super) struct Occupant {
    /// Its name within its project.
    pub(super) name: ProcessName,
    /// The icon it wears.
    pub(super) icon: ProcessIcon,
    /// What the service last reported of it, none while its project is not
    /// loaded or no service answers.
    pub(super) status: Option<ProcessStatus>,
    /// Whether a status reading has placed it yet; until then a change of
    /// its room is no change anyone saw, so it earns no pin.
    pub(super) seen: bool,
    /// Its log's last stat: none before the first, `Some(None)` while it had
    /// no log.
    pub(super) log: Option<Option<LogStat>>,
    /// Until when it speaks, while it does.
    pub(super) speaking_until: Option<Instant>,
}

impl Occupant {
    /// The process `process` of a stack, seen for the first time.
    pub(super) fn new(process: &StackedProcess) -> Self {
        Self {
            name: process.name().clone(),
            icon: *process.icon(),
            status: None,
            seen: false,
            log: None,
            speaking_until: None,
        }
    }

    /// Whether its child runs, so its log is worth statting.
    pub(super) fn runs(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| *status.state() == ProcessState::Running)
    }

    /// The process as the page draws it.
    pub(super) fn face(&self) -> FaceView {
        let status = self.status.as_ref();
        let placement = Placement::of(status);
        let usage = status.and_then(|status| status.usage().as_ref());
        FaceView::builder()
            .name(self.name.clone())
            .quoted(self.name.shell_quoted())
            .icon(self.icon)
            .room(placement.room())
            .mark(placement.mark())
            .state(status.map(|status| *status.state()).unwrap_or_default())
            .ports(status.map(listening_ports).unwrap_or_default())
            .uptime(status.and_then(|status| status.uptime().map(|uptime| uptime.as_secs())))
            .cpu(usage.map(|usage| *usage.cpu_percent()))
            .memory(usage.map(|usage| *usage.memory_bytes()))
            .restarts(status.map(|status| *status.restarts()).unwrap_or_default())
            .ended(status.and_then(|status| status.last_exit().as_ref().map(EndedView::from)))
            .attached(status.map(|status| *status.attached()).unwrap_or_default())
            .speaking(self.speaking_until.is_some())
            .build()
    }
}
