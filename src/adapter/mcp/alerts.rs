//! The crashes and failures an agent hears about on its next stack or
//! process call, without polling: per connection, the last event seen in
//! each project, and which later events are worth an alert.

use std::{collections::HashMap, sync::Mutex};

use super::{alerts_view::AlertsView, event_view::EventView};
use crate::{
    adapter::service::{EventKind, EventPage, EventSeq},
    domain::value::ProjectRoot,
};

/// Most alerts one result carries.
pub const MAX_ALERTS: usize = 5;
/// The kinds an agent is alerted to.
pub const ALERT_KINDS: [EventKind; 3] = [
    EventKind::Crashed,
    EventKind::ReadyTimeout,
    EventKind::Blocked,
];

/// Per connection, the newest event already reported in each project.
#[derive(Debug, Default)]
pub struct Alerts {
    /// The newest event seen, by project.
    seen: Mutex<HashMap<ProjectRoot, EventSeq>>,
}

impl Alerts {
    /// The newest event already reported in `project`, or none before the
    /// first call there.
    pub fn since(&self, project: &ProjectRoot) -> Option<EventSeq> {
        self.seen
            .lock()
            .ok()
            .and_then(|seen| seen.get(project).copied())
    }

    /// The alerts `page` holds for `project`, read after `read_since`, the
    /// mark [`Alerts::since`] gave when the page was asked for. The first
    /// page of a project only sets where alerts start, so history from
    /// before the connection is never replayed; a page read with a cursor of
    /// an earlier service holds that service's successor's events, every one
    /// of them new to the connection. The mark moves only forward: to the
    /// page's cursor when no other call moved it since `read_since`, or
    /// further along the same log. An event another call already reported is
    /// left out, and a page that dropped events says so.
    pub fn take(
        &self,
        project: &ProjectRoot,
        read_since: Option<EventSeq>,
        page: &EventPage,
    ) -> AlertsView {
        let Ok(mut seen) = self.seen.lock() else {
            return AlertsView::default();
        };
        let current = seen.get(project).copied();
        let cursor = *page.cursor();
        if current == read_since || current.is_some_and(|mark| cursor.is_after(mark)) {
            seen.insert(project.clone(), cursor);
        }
        drop(seen);
        if read_since.is_none() {
            return AlertsView::default();
        }
        let reported = current.filter(|mark| Some(*mark) != read_since);
        let worthy: Vec<EventView> = page
            .events()
            .iter()
            .filter(|event| ALERT_KINDS.contains(&event.kind()))
            .filter(|event| {
                reported.is_none_or(|mark| {
                    event.seq().epoch() != mark.epoch() || event.seq().is_after(mark)
                })
            })
            .map(EventView::from)
            .collect();
        let omitted = worthy.len().saturating_sub(MAX_ALERTS);
        AlertsView::builder()
            .alerts(worthy.into_iter().skip(omitted).collect())
            .alerts_omitted((omitted > 0).then_some(omitted))
            .alerts_gap((*page.gap()).then_some(true))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use super::*;
    use crate::{adapter::service::StackEvent, domain::value::ProcessName};

    /// The project every alert here is about.
    fn shop() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/shop")).unwrap()
    }

    /// The epoch of the service most pages here come from.
    const EPOCH: u32 = 3;

    /// The cursor of the `seq`th event of the log under `epoch`.
    fn cursor(epoch: u32, seq: u64) -> EventSeq {
        EventSeq::builder().epoch(epoch).seq(seq).build()
    }

    /// A page of events of `kinds`, numbered from `first`.
    fn page(first: u64, kinds: &[EventKind]) -> EventPage {
        page_of(EPOCH, first, kinds)
    }

    /// A page of the log under `epoch` of events of `kinds`, numbered from
    /// `first`.
    fn page_of(epoch: u32, first: u64, kinds: &[EventKind]) -> EventPage {
        let events: Vec<StackEvent> = kinds
            .iter()
            .zip(first..)
            .map(|(kind, seq)| {
                StackEvent::builder()
                    .seq(cursor(epoch, seq))
                    .at(SystemTime::UNIX_EPOCH)
                    .project(Some(shop()))
                    .process(Some(ProcessName::try_new("api").unwrap()))
                    .kind(*kind)
                    .detail("exit 1".to_string())
                    .cursor(None)
                    .build()
            })
            .collect();
        let last = first + kinds.len() as u64 - 1;
        EventPage::builder()
            .events(events)
            .cursor(cursor(epoch, last))
            .gap(false)
            .build()
    }

    /// Alerts whose mark in `shop` starts at event `seq` of `epoch`.
    fn marked(epoch: u32, seq: u64) -> Alerts {
        let alerts = Alerts::default();
        alerts.take(&shop(), None, &page_of(epoch, seq + 1, &[]));
        assert_eq!(alerts.since(&shop()), Some(cursor(epoch, seq)));
        alerts
    }

    /// What `alerts` reports of `page`, read after its current mark.
    fn next(alerts: &Alerts, page: &EventPage) -> AlertsView {
        alerts.take(&shop(), alerts.since(&shop()), page)
    }

    /// The first call only sets the baseline; later crashes become alerts,
    /// and lifecycle noise does not.
    #[test]
    fn the_first_page_is_a_baseline_and_later_crashes_alert() {
        let alerts = Alerts::default();
        assert_eq!(
            next(&alerts, &page(1, &[EventKind::Crashed])),
            AlertsView::default()
        );
        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH, 1)));

        let view = next(&alerts, &page(2, &[EventKind::Started, EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
        assert_eq!(view.alerts()[0].kind(), &EventKind::Crashed);
        assert_eq!(view.alerts_omitted(), None);
        assert_eq!(view.alerts_gap(), None);
    }

    /// More alerts than fit keep the newest, last, and count the rest.
    #[test]
    fn a_flood_keeps_the_newest_and_counts_the_rest() {
        let alerts = marked(EPOCH, 0);

        let view = next(&alerts, &page(1, &[EventKind::Crashed; 8]));

        assert_eq!(view.alerts().len(), MAX_ALERTS);
        assert_eq!(view.alerts().last().unwrap().seq(), "e:3:8");
        assert_eq!(view.alerts_omitted(), Some(3));
    }

    /// A page of a service started afresh moves the mark to its log, and its
    /// crashes alert even though its count starts below the old mark.
    #[test]
    fn a_restarted_service_moves_the_mark_to_its_log() {
        let alerts = marked(EPOCH, 500);

        let view = next(&alerts, &page_of(EPOCH + 1, 1, &[EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH + 1, 1)));
        let later = next(&alerts, &page_of(EPOCH + 1, 2, &[EventKind::Crashed]));
        assert_eq!(later.alerts().len(), 1);
    }

    /// Two calls that read the same mark at once never both report an
    /// event, and the mark ends at the furthest page, whichever lands first.
    #[test]
    fn concurrent_calls_never_repeat_an_alert() {
        for later_first in [false, true] {
            let alerts = marked(EPOCH, 0);
            let since = alerts.since(&shop());
            let shorter = page(1, &[EventKind::Crashed, EventKind::Crashed]);
            let longer = page(1, &[EventKind::Crashed; 3]);
            let (first, second) = if later_first {
                (&longer, &shorter)
            } else {
                (&shorter, &longer)
            };

            let one = alerts.take(&shop(), since, first);
            let two = alerts.take(&shop(), since, second);

            let reported = one.alerts().len() + two.alerts().len();
            assert_eq!(reported, 3, "later first: {later_first}");
            assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH, 3)));
        }
    }

    /// A slow call whose page came from the service before a restart never
    /// moves the mark back to that old log, so the new service's events are
    /// not alerted again; the old service's events it alone read still are.
    #[test]
    fn a_page_of_an_earlier_service_never_replaces_a_newer_mark() {
        let alerts = marked(EPOCH, 4);
        let since = alerts.since(&shop());
        let restarted = page_of(EPOCH + 1, 1, &[EventKind::Crashed]);
        let stale = page_of(EPOCH, 5, &[EventKind::Crashed]);

        assert_eq!(alerts.take(&shop(), since, &restarted).alerts().len(), 1);
        assert_eq!(alerts.take(&shop(), since, &stale).alerts().len(), 1);

        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH + 1, 1)));
        assert!(
            next(&alerts, &page_of(EPOCH + 1, 2, &[]))
                .alerts()
                .is_empty()
        );
    }

    /// A page that dropped events says alerts were missed.
    #[test]
    fn a_gap_is_told() {
        let alerts = marked(EPOCH, 0);
        let mut dropped = page(9, &[EventKind::Crashed]);
        dropped = EventPage::builder()
            .events(dropped.events().clone())
            .cursor(*dropped.cursor())
            .gap(true)
            .build();

        let view = next(&alerts, &dropped);

        assert_eq!(view.alerts_gap(), Some(true));
        assert_eq!(view.alerts().len(), 1);
    }
}
