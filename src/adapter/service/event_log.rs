use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use uuid::Uuid;

use super::{
    event_draft::EventDraft, event_kind::EventKind, event_page::EventPage, event_seq::EventSeq,
    shared_event_log::SharedEventLog, stack_event::StackEvent,
};
use crate::domain::value::ProjectRoot;

/// Most events kept in memory; the oldest is dropped to make room.
pub const MAX_EVENTS: usize = 1024;
/// Most events a read without a cursor returns, the newest ones.
pub const EVENTS_WITHOUT_SINCE: usize = 20;

/// A bounded ring of the most recent lifecycle events across every project,
/// each numbered by how many were ever recorded under this log's epoch, so a
/// reader can resume from the last one it saw and tell when it missed some,
/// or holds a cursor of another service's log.
pub struct EventLog {
    /// Names this log, so its cursors are told apart from another service's.
    epoch: u32,
    /// The most recent events, oldest first.
    ring: VecDeque<StackEvent>,
    /// How many events were ever recorded, the newest one's seq.
    recorded: u64,
    /// The newest seq dropped from the ring, by the project its event
    /// concerned, or none for an event about the service as a whole.
    dropped: HashMap<Option<ProjectRoot>, u64>,
}

impl Default for EventLog {
    /// An empty log.
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    /// An empty log under a fresh random epoch.
    pub fn new() -> Self {
        let [a, b, c, d, ..] = *Uuid::new_v4().as_bytes();
        Self::with_epoch(u32::from_be_bytes([a, b, c, d]))
    }

    /// An empty log under `epoch`.
    pub fn with_epoch(epoch: u32) -> Self {
        Self {
            epoch,
            ring: VecDeque::with_capacity(MAX_EVENTS),
            recorded: 0,
            dropped: HashMap::new(),
        }
    }

    /// An empty log, ready to share.
    pub fn shared() -> SharedEventLog {
        Arc::new(Mutex::new(Self::new()))
    }

    /// Records `draft` as the newest event, its detail made printable, and
    /// returns its seq. The oldest event is dropped once the ring is full.
    pub fn record(&mut self, draft: EventDraft) -> EventSeq {
        self.record_at(draft, SystemTime::now())
    }

    /// Records `draft` as [`Self::record`] does, as having happened `at`, or
    /// as the newest event did when `at` is older, so times never go back in
    /// cursor order.
    pub fn record_at(&mut self, draft: EventDraft, at: SystemTime) -> EventSeq {
        let at = self.ring.back().map_or(at, |newest| at.max(*newest.at()));
        self.recorded += 1;
        let seq = self.seq(self.recorded);
        if self.ring.len() == MAX_EVENTS
            && let Some(oldest) = self.ring.pop_front()
        {
            self.dropped
                .insert(oldest.project().clone(), oldest.seq().seq());
        }
        self.ring.push_back(draft.into_event(seq, at));
        seq
    }

    /// The newest event's seq, or zero before any was recorded.
    pub fn latest(&self) -> EventSeq {
        self.seq(self.recorded)
    }

    /// The held events about `project` or the whole service after `since`, of
    /// one of `kinds` (any kind when empty), oldest first. A cursor of another
    /// epoch, or past the newest event, gets every held event with a gap.
    /// Without `since`, the newest [`EVENTS_WITHOUT_SINCE`] of them.
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
            Some(since) if since.epoch() != self.epoch || since.seq() > self.recorded => {
                (relevant.cloned().collect(), true)
            },
            Some(since) => (
                relevant
                    .filter(|event| event.seq().seq() > since.seq())
                    .cloned()
                    .collect(),
                self.missed(project, since),
            ),
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

    /// Whether an event about `project`, or the whole service, recorded after
    /// `since` was dropped before it could be read.
    fn missed(&self, project: &ProjectRoot, since: EventSeq) -> bool {
        [Some(project.clone()), None]
            .iter()
            .filter_map(|key| self.dropped.get(key))
            .any(|dropped| *dropped > since.seq())
    }

    /// The cursor of the `seq`th event of this log.
    fn seq(&self, seq: u64) -> EventSeq {
        EventSeq::builder().epoch(self.epoch).seq(seq).build()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::value::ProcessName;

    /// The epoch of every test log.
    const EPOCH: u32 = 7;

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

    fn cursor(epoch: u32, seq: u64) -> EventSeq {
        EventSeq::builder().epoch(epoch).seq(seq).build()
    }

    #[test]
    fn a_read_after_a_cursor_returns_only_newer_events_of_the_project() {
        let mut log = EventLog::with_epoch(EPOCH);
        let first = log.record(draft("/shop", EventKind::Started));
        log.record(draft("/admin", EventKind::Started));
        log.record(draft("/shop", EventKind::Crashed));

        let page = log.page(&root("/shop"), Some(first), &[]);

        let kinds: Vec<EventKind> = page.events().iter().map(StackEvent::kind).collect();
        assert_eq!(kinds, [EventKind::Crashed]);
        assert_eq!(*page.cursor(), cursor(EPOCH, 3));
        assert!(!page.gap());
    }

    #[test]
    fn kinds_filter_and_a_missing_cursor_returns_the_newest() {
        let mut log = EventLog::with_epoch(EPOCH);
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
        let mut log = EventLog::with_epoch(EPOCH);
        for _ in 0..MAX_EVENTS + 2 {
            log.record(draft("/shop", EventKind::Started));
        }

        let page = log.page(&root("/shop"), Some(cursor(EPOCH, 0)), &[]);

        assert!(page.gap());
        assert_eq!(page.events().len(), MAX_EVENTS);
        assert!(!log.page(&root("/shop"), Some(cursor(EPOCH, 2)), &[]).gap());
    }

    /// Only a dropped event about the project read, or the whole service,
    /// is a gap: another project's churn is not.
    #[test]
    fn a_gap_counts_only_the_project_read() {
        let mut log = EventLog::with_epoch(EPOCH);
        let seen = log.record(draft("/shop", EventKind::Started));
        for _ in 0..=MAX_EVENTS {
            log.record(draft("/admin", EventKind::Restarting));
        }

        assert!(!log.page(&root("/shop"), Some(seen), &[]).gap());
        assert!(log.page(&root("/admin"), Some(seen), &[]).gap());
    }

    /// A cursor of another service's log, or one past the newest event,
    /// gets every held event of the project with a gap.
    #[test]
    fn a_foreign_or_future_cursor_gets_the_held_events_with_a_gap() {
        let mut log = EventLog::with_epoch(EPOCH);
        log.record(draft("/shop", EventKind::Started));
        log.record(draft("/shop", EventKind::Crashed));

        for since in [cursor(EPOCH + 1, 1), cursor(EPOCH, 9)] {
            let page = log.page(&root("/shop"), Some(since), &[]);
            assert!(page.gap(), "{since}");
            assert_eq!(page.events().len(), 2, "{since}");
            assert_eq!(*page.cursor(), cursor(EPOCH, 2));
        }
    }

    /// An event recorded as having happened before the newest one takes the
    /// newest one's time, so times follow cursor order.
    #[test]
    fn times_never_go_back_in_cursor_order() {
        let mut log = EventLog::with_epoch(EPOCH);
        log.record(draft("/shop", EventKind::Started));
        log.record_at(draft("/shop", EventKind::Notice), SystemTime::UNIX_EPOCH);

        let page = log.page(&root("/shop"), None, &[]);
        let times: Vec<SystemTime> = page.events().iter().map(|event| *event.at()).collect();
        assert!(times[1] >= times[0], "{times:?}");
    }
}
