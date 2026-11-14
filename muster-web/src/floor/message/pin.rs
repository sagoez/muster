use muster_domain::process::{ProcessState, Readiness};
use muster_protocol::service::ProcessStatus;
use serde::Serialize;

use crate::floor::view::{Placement, Room};

/// What drops on a process's face the moment it changes room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pin {
    /// It came into Running.
    Ok,
    /// It is waiting to start again.
    Again,
    /// It came into Failed.
    Fail,
    /// It runs, but its readiness check ran out of time.
    Late,
}

impl Pin {
    /// The pin a process earns going from `before` to `after`, none when its
    /// change shows no pin: into Starting or Off, or a stop walking through
    /// Restarting's room.
    #[must_use]
    pub fn between(before: Option<&ProcessStatus>, after: Option<&ProcessStatus>) -> Option<Self> {
        let (was, now) = (Placement::of(before).room(), Placement::of(after).room());
        let ready = |status: Option<&ProcessStatus>| status.map(|status| *status.ready());
        let state = |status: Option<&ProcessStatus>| status.map(|status| *status.state());
        let timed_out = Some(Readiness::TimedOut);
        let restarting = Some(ProcessState::Restarting);
        match now {
            Room::Failed if was != Room::Failed => Some(Self::Fail),
            Room::Running if ready(after) == timed_out && ready(before) != timed_out => {
                Some(Self::Late)
            },
            Room::Running if was != Room::Running => Some(Self::Ok),
            Room::Restarting if state(after) == restarting && state(before) != restarting => {
                Some(Self::Again)
            },
            _ => None,
        }
    }
}
