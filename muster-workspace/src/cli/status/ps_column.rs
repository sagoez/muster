//! The columns of the `muster ps` table: each one's header and the cell it
//! shows for a process, read from one place so they never fall out of step.

use bytesize::ByteSize;
use muster_client::listening_ports;
use muster_domain::{process::Readiness, value::printable};
use muster_protocol::service::ProcessStatus;
use strum::{EnumIter, EnumMessage};

use crate::cli::{
    rows::health::Health,
    status::output::{cell, short_duration},
};

/// The READY cell of a run that passed its readiness check.
const READY_YES: &str = "yes";
/// The READY cell of a run still waiting on its readiness check.
const READY_WAITING: &str = "waiting";
/// The READY cell of a run that did not pass its readiness check in time.
const READY_TIMEOUT: &str = "timeout";
/// Between the ports in a PORTS cell.
const PORT_LIST_SEPARATOR: &str = ",";

/// A column of the `ps` table, in the order the table shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumMessage)]
pub(super) enum PsColumn {
    /// The process's name.
    #[strum(message = "NAME")]
    Name,
    /// Its lifecycle state.
    #[strum(message = "STATE")]
    State,
    /// Whether its run passed its readiness check.
    #[strum(message = "READY")]
    Ready,
    /// The ports its process tree listens on.
    #[strum(message = "PORTS")]
    Ports,
    /// Its pid.
    #[strum(message = "PID")]
    Pid,
    /// How long its run has been up.
    #[strum(message = "UPTIME")]
    Uptime,
    /// How often it restarted.
    #[strum(message = "RESTARTS")]
    Restarts,
    /// Its CPU use.
    #[strum(message = "CPU")]
    Cpu,
    /// Its memory use.
    #[strum(message = "MEM")]
    Mem,
    /// How many terminals are attached to it.
    #[strum(message = "ATTACHED")]
    Attached,
    /// How its last run ended.
    #[strum(message = "LAST EXIT")]
    LastExit,
}

impl PsColumn {
    /// The column's header.
    pub(super) fn header(self) -> &'static str {
        self.get_message().unwrap_or_default()
    }

    /// What this column shows for `process`, made printable.
    pub(super) fn cell(self, process: &ProcessStatus) -> String {
        let usage = process.usage().as_ref();
        match self {
            Self::Name => printable(process.name().as_ref()),
            Self::State => Health::of_lifecycle(process).to_string(),
            Self::Ready => cell(ready_cell(*process.ready())),
            Self::Ports => cell(ports_cell(process)),
            Self::Pid => cell(process.pid().map(|pid| pid.to_string())),
            Self::Uptime => cell(process.uptime().map(short_duration)),
            Self::Restarts => process.restarts().to_string(),
            Self::Cpu => cell(usage.map(|usage| format!("{:.1}%", usage.cpu_percent()))),
            Self::Mem => cell(usage.map(|usage| ByteSize::b(*usage.memory_bytes()).to_string())),
            Self::Attached => {
                cell((*process.attached() > 0).then(|| process.attached().to_string()))
            },
            Self::LastExit => cell(
                process
                    .last_exit()
                    .as_ref()
                    .map(|exit| printable(&exit.outcome().to_string())),
            ),
        }
    }
}

/// Whether a process's run is ready, in a `ps` cell: `yes`, `waiting`,
/// `timeout`, or nothing when it has no check.
fn ready_cell(ready: Readiness) -> Option<String> {
    match ready {
        Readiness::None => None,
        Readiness::Waiting => Some(READY_WAITING.to_string()),
        Readiness::Ready => Some(READY_YES.to_string()),
        Readiness::TimedOut => Some(READY_TIMEOUT.to_string()),
    }
}

/// The ports a process tree listens on, in a `ps` cell: `3000,5173`, or
/// nothing when it listens on none.
fn ports_cell(process: &ProcessStatus) -> Option<String> {
    let ports = listening_ports(process);
    (!ports.is_empty()).then(|| {
        ports
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(PORT_LIST_SEPARATOR)
    })
}
