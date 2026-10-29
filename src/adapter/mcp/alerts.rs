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
    /// moving backwards.
    pub fn mark_seen(&self, project: &ProjectRoot, cursor: EventSeq) {
        if let Ok(mut seen) = self.seen.lock() {
            let entry = seen.entry(project.clone()).or_insert(cursor);
            *entry = (*entry).max(cursor);
        }
    }

    /// The alerts `page` holds for `project`, read after [`Alerts::since`],
    /// marking it seen. The first page of a project only sets where alerts
    /// start, so history from before the connection is never replayed.
    pub fn take(&self, project: &ProjectRoot, page: &EventPage) -> AlertsView {
        let since = self.since(project);
        if since.is_some_and(|since| *page.cursor() < since)
            && let Ok(mut seen) = self.seen.lock()
        {
            // A service started afresh numbers its events from zero again.
            seen.insert(project.clone(), *page.cursor());
            return AlertsView::default();
        }
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

    /// A page of events of `kinds`, numbered from `first`.
    fn page(first: u64, kinds: &[EventKind]) -> EventPage {
        let events: Vec<StackEvent> = kinds
            .iter()
            .zip(first..)
            .map(|(kind, seq)| {
                StackEvent::builder()
                    .seq(EventSeq::new(seq))
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
            .cursor(EventSeq::new(last))
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
        assert_eq!(alerts.since(&shop()), Some(EventSeq::new(2)));

        let view = alerts.take(&shop(), &page(2, &[EventKind::Started, EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
        assert_eq!(view.alerts()[0].kind(), &EventKind::Crashed);
        assert_eq!(view.alerts_omitted(), None);
    }

    /// More alerts than fit keep the newest, last, and count the rest.
    #[test]
    fn a_flood_keeps_the_newest_and_counts_the_rest() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), EventSeq::new(0));

        let view = alerts.take(&shop(), &page(1, &[EventKind::Crashed; 8]));

        assert_eq!(view.alerts().len(), MAX_ALERTS);
        assert_eq!(view.alerts().last().unwrap().seq(), "e:8");
        assert_eq!(view.alerts_omitted(), Some(3));
    }

    /// A page older than the mark comes from a service started afresh, so
    /// the mark restarts there and later crashes still alert.
    #[test]
    fn a_restarted_service_resets_the_mark() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), EventSeq::new(500));

        assert_eq!(alerts.take(&shop(), &page(1, &[])), AlertsView::default());
        assert_eq!(alerts.since(&shop()), Some(EventSeq::new(1)));
        let view = alerts.take(&shop(), &page(2, &[EventKind::Crashed]));
        assert_eq!(view.alerts().len(), 1);
    }

    /// The seen mark never moves backwards.
    #[test]
    fn the_seen_mark_only_advances() {
        let alerts = Alerts::default();
        alerts.mark_seen(&shop(), EventSeq::new(9));
        alerts.mark_seen(&shop(), EventSeq::new(4));
        assert_eq!(alerts.since(&shop()), Some(EventSeq::new(9)));
    }
}
