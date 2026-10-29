use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    event_kind::EventKind, event_page::EventPage, event_seq::EventSeq, stack_event::StackEvent,
};
use crate::{
    adapter::sanitize::printable,
    domain::value::{OutputCursor, ProcessName, ProjectRoot},
};

/// Most events kept in memory; the oldest is dropped to make room.
pub const MAX_EVENTS: usize = 1024;
/// Most events a read without a cursor returns, the newest ones.
pub const EVENTS_WITHOUT_SINCE: usize = 20;

/// The event log shared between the supervisors that record events and the
/// loop that answers reads of it.
pub type SharedEventLog = Arc<Mutex<EventLog>>;

/// A bounded ring of the most recent lifecycle events across every project,
/// each numbered by how many were ever recorded, so a reader can resume from
/// the last one it saw and tell when it missed some.
pub struct EventLog {
    /// The most recent events, oldest first.
    ring: VecDeque<StackEvent>,
    /// How many events were ever recorded, the newest one's seq.
    recorded: u64,
}

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

impl Default for EventLog {
    /// An empty log.
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    /// An empty log.
    pub fn new() -> Self {
        Self {
            ring: VecDeque::with_capacity(MAX_EVENTS),
            recorded: 0,
        }
    }

    /// An empty log, ready to share.
    pub fn shared() -> SharedEventLog {
        Arc::new(Mutex::new(Self::new()))
    }

    /// Records `draft` as the newest event, its detail made printable, and
    /// returns its seq. The oldest event is dropped once the ring is full.
    pub fn record(&mut self, draft: EventDraft) -> EventSeq {
        let EventDraft {
            project,
            process,
            kind,
            detail,
            cursor,
        } = draft;
        self.recorded += 1;
        let seq = EventSeq::new(self.recorded);
        if self.ring.len() == MAX_EVENTS {
            self.ring.pop_front();
        }
        self.ring.push_back(
            StackEvent::builder()
                .seq(seq)
                .at(SystemTime::now())
                .project(project)
                .process(process)
                .kind(kind)
                .detail(printable(&detail))
                .cursor(cursor)
                .build(),
        );
        seq
    }

    /// The newest event's seq, or zero before any was recorded.
    pub fn latest(&self) -> EventSeq {
        EventSeq::new(self.recorded)
    }

    /// The held events about `project` or the whole service after `since`, of
    /// one of `kinds` (any kind when empty), oldest first. Without `since`,
    /// the newest [`EVENTS_WITHOUT_SINCE`] of them.
    pub fn page(
        &self,
        project: &ProjectRoot,
        since: Option<EventSeq>,
        kinds: &[EventKind],
    ) -> EventPage {
        let relevant = self.ring.iter().filter(|event| {
            event.project().as_ref().is_none_or(|root| root == project)
                && (kinds.is_empty() || kinds.contains(&event.kind()))
        });
        let (events, gap) = match since {
            Some(since) => {
                let oldest = self.ring.front().map(StackEvent::seq);
                let gap = oldest.is_some_and(|oldest| oldest.into_inner() > since.into_inner() + 1);
                (
                    relevant
                        .filter(|event| event.seq() > since)
                        .cloned()
                        .collect(),
                    gap,
                )
            },
            None => {
                let mut newest: Vec<StackEvent> =
                    relevant.rev().take(EVENTS_WITHOUT_SINCE).cloned().collect();
                newest.reverse();
                (newest, false)
            },
        };
        EventPage::builder()
            .events(events)
            .cursor(self.latest())
            .gap(gap)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn root(path: &str) -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from(path)).unwrap()
    }

    fn draft(project: &str, kind: EventKind) -> EventDraft {
        EventDraft::builder()
            .project(Some(root(project)))
            .process(Some(ProcessName::try_new("api").unwrap()))
            .kind(kind)
            .detail("exit 1")
            .build()
    }

    #[test]
    fn a_read_after_a_cursor_returns_only_newer_events_of_the_project() {
        let mut log = EventLog::new();
        let first = log.record(draft("/shop", EventKind::Started));
        log.record(draft("/admin", EventKind::Started));
        log.record(draft("/shop", EventKind::Crashed));

        let page = log.page(&root("/shop"), Some(first), &[]);

        let kinds: Vec<EventKind> = page.events().iter().map(StackEvent::kind).collect();
        assert_eq!(kinds, [EventKind::Crashed]);
        assert_eq!(*page.cursor(), EventSeq::new(3));
        assert!(!page.gap());
    }

    #[test]
    fn kinds_filter_and_a_missing_cursor_returns_the_newest() {
        let mut log = EventLog::new();
        for _ in 0..EVENTS_WITHOUT_SINCE + 5 {
            log.record(draft("/shop", EventKind::Restarting));
        }
        log.record(draft("/shop", EventKind::Crashed));

        let crashes = log.page(&root("/shop"), None, &[EventKind::Crashed]);
        let all = log.page(&root("/shop"), None, &[]);

        assert_eq!(crashes.events().len(), 1);
        assert_eq!(all.events().len(), EVENTS_WITHOUT_SINCE);
        assert_eq!(all.events().last().unwrap().kind(), EventKind::Crashed);
    }

    #[test]
    fn a_cursor_older_than_the_ring_reports_a_gap() {
        let mut log = EventLog::new();
        for _ in 0..MAX_EVENTS + 2 {
            log.record(draft("/shop", EventKind::Started));
        }

        let page = log.page(&root("/shop"), Some(EventSeq::new(0)), &[]);

        assert!(page.gap());
        assert_eq!(page.events().len(), MAX_EVENTS);
        assert!(!log.page(&root("/shop"), Some(EventSeq::new(2)), &[]).gap());
    }
}
