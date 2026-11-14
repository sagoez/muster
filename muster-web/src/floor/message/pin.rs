use muster_domain::process::{ProcessState, Readiness};
use muster_protocol::service::{EventKind, ProcessStatus};
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

    /// The pin an event of `kind` stands for, which a status poll may have
    /// missed, resolved against `now`, the status read after it: a restart
    /// is again, a crash fail, a passed check ok, and a start ok only when
    /// `now` runs with no check to pass. Other kinds pin nothing.
    #[must_use]
    pub fn heard(kind: EventKind, now: Option<&ProcessStatus>) -> Option<Self> {
        let runs_unchecked = now.is_some_and(|status| {
            *status.state() == ProcessState::Running && *status.ready() == Readiness::None
        });
        match kind {
            EventKind::Restarting => Some(Self::Again),
            EventKind::Crashed => Some(Self::Fail),
            EventKind::Ready => Some(Self::Ok),
            EventKind::Started if runs_unchecked => Some(Self::Ok),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floor::fixture::process_status;

    /// A restart, a crash and a passed check pin whatever follows; a start
    /// pins ok only into a run with no check, and other kinds pin nothing.
    #[test]
    fn each_heard_kind_resolves_to_its_pin() {
        let running = process_status("web", ProcessState::Running, Readiness::None);
        let waiting = process_status("web", ProcessState::Running, Readiness::Waiting);
        let crashed = process_status("web", ProcessState::Crashed, Readiness::None);

        assert_eq!(
            Pin::heard(EventKind::Restarting, Some(&running)),
            Some(Pin::Again)
        );
        assert_eq!(
            Pin::heard(EventKind::Crashed, Some(&running)),
            Some(Pin::Fail)
        );
        assert_eq!(Pin::heard(EventKind::Ready, Some(&running)), Some(Pin::Ok));
        assert_eq!(
            Pin::heard(EventKind::Started, Some(&running)),
            Some(Pin::Ok)
        );
        assert_eq!(Pin::heard(EventKind::Started, Some(&waiting)), None);
        assert_eq!(Pin::heard(EventKind::Started, Some(&crashed)), None);
        assert_eq!(Pin::heard(EventKind::Started, None), None);
        assert_eq!(Pin::heard(EventKind::Stopped, Some(&running)), None);
        assert_eq!(Pin::heard(EventKind::Exited, Some(&crashed)), None);
    }
}
