use std::time::SystemTime;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{event_kind::EventKind, event_seq::EventSeq, stack_event::StackEvent};
use crate::{
    adapter::sanitize::printable,
    domain::value::{OutputCursor, ProcessName, ProjectRoot},
};

/// What one event is about and what it says, before the log numbers it.
#[derive(Debug, Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct EventDraft {
    /// The project it concerns, or none for the service as a whole.
    project: Option<ProjectRoot>,
    /// The process it concerns, when one.
    #[builder(default)]
    process: Option<ProcessName>,
    /// What happened.
    kind: EventKind,
    /// More about it, for a person to read.
    #[builder(setter(into))]
    detail: String,
    /// Where the process's output stood when it happened.
    #[builder(default)]
    cursor: Option<OutputCursor>,
}

impl EventDraft {
    /// The event this draft is once the log numbers it `seq`, as having
    /// happened `at`, its detail made printable.
    pub(super) fn into_event(self, seq: EventSeq, at: SystemTime) -> StackEvent {
        let Self {
            project,
            process,
            kind,
            detail,
            cursor,
        } = self;
        StackEvent::builder()
            .seq(seq)
            .at(at)
            .project(project)
            .process(process)
            .kind(kind)
            .detail(printable(&detail))
            .cursor(cursor)
            .build()
    }
}
