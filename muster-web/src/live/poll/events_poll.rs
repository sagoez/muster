use std::{sync::Arc, time::Duration};

use muster_client::{Asked, ServiceAccess};
use muster_protocol::service::{ServiceRequest, ServiceResponse};

use crate::{
    floor::EventsReading,
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// Longest the service holds one read of its events open waiting for one.
pub const EVENTS_WAIT: Duration = Duration::from_secs(30);
/// How long a read that found no service, or failed, waits before the next.
pub const EVENTS_RETRY: Duration = Duration::from_secs(1);

/// Reads every loaded project's events from `service` for as long as the
/// hub is there, each read held open up to [`EVENTS_WAIT`] and resumed from
/// the last one's cursor. A page read after a cursor goes to `hub`; one
/// read from none holds only history, so it only gives the cursor. A read
/// that finds no service or fails starts over from no cursor after
/// [`EVENTS_RETRY`]. It asks only a service that runs, never starting one,
/// and loads no project.
pub fn poll_events(service: &Arc<dyn ServiceAccess>, hub: &HubHandle, pause: &Pause) {
    let mut since = None;
    loop {
        let request = ServiceRequest::Events {
            project: None,
            since,
            kinds: Vec::new(),
            wait: EVENTS_WAIT,
        };
        match service.ask_waiting_if_running(&request, EVENTS_WAIT) {
            Ok(Asked::Answered(ServiceResponse::Events(page))) => {
                let resumed = since.replace(*page.cursor()).is_some();
                let reading = EventsReading::builder()
                    .events(page.events().clone())
                    .gap(*page.gap())
                    .build();
                if resumed && !hub.send(HubEvent::Events(reading)) {
                    return;
                }
            },
            _ => {
                since = None;
                if !pause.wait(EVENTS_RETRY) {
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use crossbeam_channel::{bounded, unbounded};
    use muster_client::fake_service::FakeService;
    use muster_protocol::service::{EventKind, EventPage, EventSeq};

    use super::*;
    use crate::floor::fixture::stack_event;

    /// Longest the test waits for a page to reach the hub.
    const BOUND: Duration = Duration::from_secs(10);

    /// The cursor numbered `seq`.
    fn cursor(seq: u64) -> EventSeq {
        EventSeq::builder().epoch(1).seq(seq).build()
    }

    /// The service's answer of a page holding one crash of shop's `web`,
    /// ending at `seq`.
    fn page(seq: u64) -> Asked {
        Asked::Answered(ServiceResponse::Events(
            EventPage::builder()
                .events(vec![stack_event("shop", "web", EventKind::Crashed)])
                .cursor(cursor(seq))
                .gap(false)
                .build(),
        ))
    }

    /// The cursor each events read asked of `service` resumed from.
    fn cursors(asked: &[ServiceRequest]) -> Vec<Option<EventSeq>> {
        asked
            .iter()
            .map(|request| match request {
                ServiceRequest::Events { since, .. } => *since,
                other => panic!("not an events read: {other:?}"),
            })
            .collect()
    }

    /// The long poll resumes from each page's cursor, sends only pages read
    /// after one, and starts over from no cursor once a read finds no
    /// service.
    #[test]
    fn the_long_poll_resumes_from_its_cursor_and_restarts_after_a_failure() {
        let service = Arc::new(FakeService::default().asked_in_turn(vec![
            page(1),
            page(2),
            Asked::Absent,
            page(3),
            page(4),
        ]));
        let (sender, events) = unbounded();
        let (_life, lifeline) = bounded::<()>(0);
        let hub = HubHandle::new(sender);
        let polled: Arc<dyn ServiceAccess> = service.clone();
        let poller = thread::spawn(move || poll_events(&polled, &hub, &Pause::new(lifeline)));

        let first = events.recv_timeout(BOUND).unwrap();
        let second = events.recv_timeout(BOUND).unwrap();
        drop(events);
        poller.join().unwrap();

        for sent in [first, second] {
            let HubEvent::Events(reading) = sent else {
                panic!("not an events page");
            };
            assert_eq!(reading.events().len(), 1);
            assert!(!reading.gap());
        }
        let asked = cursors(&service.asked());
        assert_eq!(asked[..5], [
            None,
            Some(cursor(1)),
            Some(cursor(2)),
            None,
            Some(cursor(3)),
        ]);
    }
}
