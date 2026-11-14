use getset::CopyGetters;
use muster_domain::process::{ProcessState, Readiness};
use muster_protocol::service::ProcessStatus;
use typed_builder::TypedBuilder;

use super::{mark::Mark, room::Room};

/// Where a process stands on its floor: its room and the mark it wears there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct Placement {
    /// The room it stands in.
    room: Room,
    /// What sets it apart in that room, if anything.
    mark: Option<Mark>,
}

impl Placement {
    /// Where a process whose service reports `status` stands. A process with
    /// no status, of a project the service has not loaded, is off.
    #[must_use]
    pub fn of(status: Option<&ProcessStatus>) -> Self {
        let Some(status) = status else {
            return Self::builder().room(Room::Off).mark(None).build();
        };
        let (room, mark) = match (*status.state(), *status.ready()) {
            (ProcessState::Starting, _) | (ProcessState::Running, Readiness::Waiting) => {
                (Room::Starting, None)
            },
            (ProcessState::Running, _) => (Room::Running, None),
            (ProcessState::Pending | ProcessState::Exited, _) => (Room::Off, None),
            (ProcessState::Paused, _) => (Room::Off, Some(Mark::Paused)),
            (ProcessState::Stopping, _) => (Room::Restarting, Some(Mark::Quiet)),
            (ProcessState::Restarting, _) => (Room::Restarting, None),
            (ProcessState::Unsupervised, _) => (Room::Running, Some(Mark::Muted)),
            (ProcessState::Blocked | ProcessState::Crashed, _) => (Room::Failed, None),
        };
        Self::builder().room(room).mark(mark).build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floor::fixture::process_status;

    /// Where a process in `state` with readiness `ready` stands.
    fn placed(state: ProcessState, ready: Readiness) -> (Room, Option<Mark>) {
        let placement = Placement::of(Some(&process_status("web", state, ready)));
        (placement.room(), placement.mark())
    }

    /// Every state lands in its room, with its mark.
    #[test]
    fn every_state_lands_in_its_room() {
        let cases = [
            (
                ProcessState::Starting,
                Readiness::None,
                Room::Starting,
                None,
            ),
            (
                ProcessState::Running,
                Readiness::Waiting,
                Room::Starting,
                None,
            ),
            (ProcessState::Running, Readiness::None, Room::Running, None),
            (ProcessState::Running, Readiness::Ready, Room::Running, None),
            (
                ProcessState::Running,
                Readiness::TimedOut,
                Room::Running,
                None,
            ),
            (ProcessState::Pending, Readiness::None, Room::Off, None),
            (ProcessState::Exited, Readiness::None, Room::Off, None),
            (
                ProcessState::Paused,
                Readiness::None,
                Room::Off,
                Some(Mark::Paused),
            ),
            (
                ProcessState::Stopping,
                Readiness::None,
                Room::Restarting,
                Some(Mark::Quiet),
            ),
            (
                ProcessState::Restarting,
                Readiness::None,
                Room::Restarting,
                None,
            ),
            (
                ProcessState::Unsupervised,
                Readiness::None,
                Room::Running,
                Some(Mark::Muted),
            ),
            (ProcessState::Blocked, Readiness::None, Room::Failed, None),
            (ProcessState::Crashed, Readiness::None, Room::Failed, None),
        ];
        for (state, ready, room, mark) in cases {
            assert_eq!(placed(state, ready), (room, mark), "{state} {ready}");
        }
    }

    /// A process the service does not report is off, unmarked.
    #[test]
    fn a_process_without_status_is_off() {
        let placement = Placement::of(None);
        assert_eq!((placement.room(), placement.mark()), (Room::Off, None));
    }
}
