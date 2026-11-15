use std::time::SystemTime;

use getset::Getters;
use muster_domain::value::{OutputCursor, ProcessName, ProjectRoot, printable};
use muster_protocol::service::{EventKind, EventSeq, StackEvent};
use typed_builder::TypedBuilder;

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
    /// For a start, whether the run armed a readiness check.
    #[builder(default)]
    waits_ready: Option<bool>,
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
            waits_ready,
        } = self;
        StackEvent::builder()
            .seq(seq)
            .at(at)
            .project(project)
            .process(process)
            .kind(kind)
            .detail(printable(&detail))
            .cursor(cursor)
            .waits_ready(waits_ready)
            .build()
    }
}
