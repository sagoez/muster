use muster_domain::process::ProcessState;
use serde::Serialize;
use strum::IntoStaticStr;

use super::{mark::Mark, placement::Placement, room::Room};

/// What the command a card suggests does, named as its `muster` subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, IntoStaticStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum NextVerb {
    /// Shows why its last run failed.
    Logs,
    /// Shows why the service would not start it, as a blocked run logged
    /// nothing of its own.
    Notices,
    /// Starts it.
    Start,
    /// Resumes its suspended child.
    Resume,
    /// Stops a child the service can no longer reach.
    Stop,
    /// Shows its terminal.
    Attach,
}

impl NextVerb {
    /// The verb for a process standing at `placement` in `state`.
    #[must_use]
    pub(crate) fn of(placement: Placement, state: ProcessState) -> Self {
        match (placement.room(), placement.mark()) {
            _ if state == ProcessState::Blocked => Self::Notices,
            (Room::Failed, _) => Self::Logs,
            (_, Some(Mark::Paused)) => Self::Resume,
            (Room::Off, _) => Self::Start,
            (_, Some(Mark::Muted)) => Self::Stop,
            _ => Self::Attach,
        }
    }

    /// Whether the command names the process, or only its project.
    pub(crate) fn names_the_process(self) -> bool {
        self != Self::Notices
    }
}
