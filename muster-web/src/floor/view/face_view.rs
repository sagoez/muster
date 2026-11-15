use getset::Getters;
use muster_domain::{
    process::{ProcessIcon, ProcessState},
    value::ProcessName,
};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    ended_view::EndedView, mark::Mark, next_step::NextStep, room::Room, unconfirmed::Unconfirmed,
};

/// One process as the page draws its face and card.
#[derive(Clone, Debug, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct FaceView {
    /// Its name within its project.
    name: ProcessName,
    /// The command its card suggests; none for a name no shell word can
    /// carry.
    next: Option<NextStep>,
    /// The mark on its face.
    #[serde(with = "muster_protocol::wire::form")]
    icon: ProcessIcon,
    /// The room it stands in.
    room: Room,
    /// What sets it apart in that room.
    mark: Option<Mark>,
    /// Why nothing has confirmed its status since the view last asked, so
    /// the page draws it dimmed, as last known; none once confirmed.
    unconfirmed: Option<Unconfirmed>,
    /// Its lifecycle state.
    #[serde(with = "muster_protocol::wire::form")]
    state: ProcessState,
    /// The TCP ports its process tree listens on, ascending, each once.
    ports: Vec<u16>,
    /// When its child started, in seconds since the Unix epoch; the page
    /// counts the uptime from it.
    started: Option<u64>,
    /// CPU use of its process tree, as a percentage where 100 is one core.
    cpu: Option<f32>,
    /// Resident memory of its process tree, in bytes.
    memory: Option<u64>,
    /// Times it was restarted.
    restarts: u32,
    /// How its last run ended, while it is not running.
    ended: Option<EndedView>,
    /// Writable clients attached to its terminal.
    attached: u32,
    /// Whether its log grew in the last moments.
    speaking: bool,
}

impl FaceView {
    /// Whether the page draws `other` otherwise than this, its usage and
    /// whether it speaks aside: those change on their own schedule and
    /// messages.
    #[must_use]
    pub(crate) fn differs_beyond_usage(&self, other: &Self) -> bool {
        self.without_usage() != other.without_usage()
    }

    /// Whether `other` shows another CPU or memory use than this.
    #[must_use]
    pub(crate) fn usage_differs(&self, other: &Self) -> bool {
        self.cpu != other.cpu || self.memory != other.memory
    }

    /// This face with no usage, not speaking.
    fn without_usage(&self) -> Self {
        Self {
            cpu: None,
            memory: None,
            speaking: false,
            ..self.clone()
        }
    }
}
