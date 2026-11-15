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
    /// messages. It compares field by field, copying nothing, and names
    /// every field so a new one cannot be left out.
    #[must_use]
    pub(crate) fn differs_beyond_usage(&self, other: &Self) -> bool {
        let Self {
            name,
            next,
            icon,
            room,
            mark,
            unconfirmed,
            state,
            ports,
            started,
            cpu: _,
            memory: _,
            restarts,
            ended,
            attached,
            speaking: _,
        } = self;
        *name != other.name
            || *next != other.next
            || *icon != other.icon
            || *room != other.room
            || *mark != other.mark
            || *unconfirmed != other.unconfirmed
            || *state != other.state
            || *ports != other.ports
            || *started != other.started
            || *restarts != other.restarts
            || *ended != other.ended
            || *attached != other.attached
    }

    /// Whether `other` shows another CPU or memory use than this.
    #[must_use]
    pub(crate) fn usage_differs(&self, other: &Self) -> bool {
        self.cpu != other.cpu || self.memory != other.memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face of `web`, running, with the usage `cpu` and `memory`, speaking
    /// when `speaking`, after `restarts` restarts.
    fn face(cpu: Option<f32>, memory: Option<u64>, speaking: bool, restarts: u32) -> FaceView {
        FaceView::builder()
            .name(ProcessName::try_new("web").unwrap())
            .next(None)
            .icon(ProcessIcon::Terminal)
            .room(Room::Running)
            .mark(None)
            .unconfirmed(None)
            .state(ProcessState::Running)
            .ports(vec![8080])
            .started(Some(100))
            .cpu(cpu)
            .memory(memory)
            .restarts(restarts)
            .ended(None)
            .attached(0)
            .speaking(speaking)
            .build()
    }

    /// Usage and speaking alone are no difference beyond usage; any other
    /// field is, and usage is told apart on its own.
    #[test]
    fn only_usage_and_speaking_are_set_aside() {
        let shown = face(Some(1.0), Some(10), false, 0);

        let busier = face(Some(50.0), Some(99), true, 0);
        let restarted = face(Some(1.0), Some(10), false, 1);

        assert!(!shown.differs_beyond_usage(&busier));
        assert!(shown.usage_differs(&busier));
        assert!(shown.differs_beyond_usage(&restarted));
        assert!(!shown.usage_differs(&restarted));
        assert!(!shown.differs_beyond_usage(&shown.clone()));
    }
}
