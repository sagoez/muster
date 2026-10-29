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

    /// Records that every event of `project` up to `cursor` was seen, never
    /// moving backwards within one service's log; a cursor of a newer
    /// service's log replaces the mark.
    fn mark_seen(&self, project: &ProjectRoot, cursor: EventSeq) {
        if let Ok(mut seen) = self.seen.lock() {
            let entry = seen.entry(project.clone()).or_insert(cursor);
            if cursor.is_after(*entry) || cursor.epoch() != entry.epoch() {
                *entry = cursor;
            }
        }
    }

    /// The alerts `page` holds for `project`, read after [`Alerts::since`],
    /// marking it seen. The first page of a project only sets where alerts
    /// start, so history from before the connection is never replayed; a page
    /// read with a cursor of an earlier service holds that service's
    /// successor's events, every one of them new to the connection.
    pub fn take(&self, project: &ProjectRoot, page: &EventPage) -> AlertsView {
        let since = self.since(project);
        self.mark_seen(project, *page.cursor());
        if since.is_none() {
            return AlertsView::default();
        }
        let worthy: Vec<EventView> = page
            .events()
            .iter()
            .filter(|event| ALERT_KINDS.contains(&event.kind()))
            .map(EventView::from)
            .collect();
        let omitted = worthy.len().saturating_sub(MAX_ALERTS);
        AlertsView::builder()
            .alerts(worthy.into_iter().skip(omitted).collect())
            .alerts_omitted((omitted > 0).then_some(omitted))
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
        let last = first + kinds.len() as u64;
        EventPage::builder()
            .events(events)
            .cursor(cursor(epoch, last))
            .gap(false)
            .build()
    }

    /// The first call only sets the baseline; later crashes become alerts,
    /// and lifecycle noise does not.
    #[test]
    fn the_first_page_is_a_baseline_and_later_crashes_alert() {
        let alerts = Alerts::default();
        assert_eq!(
            alerts.take(&shop(), &page(1, &[EventKind::Crashed])),
            AlertsView::default()
        );
        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH, 2)));

        let view = alerts.take(&shop(), &page(2, &[EventKind::Started, EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
        assert_eq!(view.alerts()[0].kind(), &EventKind::Crashed);
        assert_eq!(view.alerts_omitted(), None);
    }

    /// More alerts than fit keep the newest, last, and count the rest.
    #[test]
    fn a_flood_keeps_the_newest_and_counts_the_rest() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), cursor(EPOCH, 0));

        let view = alerts.take(&shop(), &page(1, &[EventKind::Crashed; 8]));

        assert_eq!(view.alerts().len(), MAX_ALERTS);
        assert_eq!(view.alerts().last().unwrap().seq(), "e:3:8");
        assert_eq!(view.alerts_omitted(), Some(3));
    }

    /// A page of a service started afresh moves the mark to its log, and its
    /// crashes alert even though its count starts below the old mark.
    #[test]
    fn a_restarted_service_moves_the_mark_to_its_log() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), cursor(EPOCH, 500));

        let view = alerts.take(&shop(), &page_of(EPOCH + 1, 1, &[EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH + 1, 2)));
        let later = alerts.take(&shop(), &page_of(EPOCH + 1, 2, &[EventKind::Crashed]));
        assert_eq!(later.alerts().len(), 1);
    }

    /// The seen mark never moves backwards.
    #[test]
    fn the_seen_mark_only_advances() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), cursor(EPOCH, 9));
        alerts.mark_seen(&shop(), cursor(EPOCH, 4));
        assert_eq!(alerts.since(&shop()), Some(cursor(EPOCH, 9)));
    }
}
