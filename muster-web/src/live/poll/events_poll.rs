use std::{mem, sync::Arc, time::Duration};

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
/// hub is there, each read resumed from the last one's cursor and held open
/// up to [`EVENTS_WAIT`]. A page read after a cursor goes to `hub`. A read
/// from no cursor holds only history, so it only gives the cursor, and is
/// asked not to wait: a service with nothing logged yet would hold it open
/// and answer with the very events it would then drop. A read that finds no
/// service or fails starts over from no cursor after [`EVENTS_RETRY`], and
/// once the read after that finds a cursor again, `hub` is told of a gap,
/// since what happened meanwhile was never read. After each read it parks
/// on `pause` while no page streams, and resumes from its cursor once one
/// does, the hub marking the gap a park leaves. It asks only a service
/// that runs, never starting one, and loads no project.
pub fn poll_events(service: &Arc<dyn ServiceAccess>, hub: &HubHandle, pause: &Pause) {
    let mut since = None;
    let mut lost = false;
    loop {
        let request = ServiceRequest::Events {
            project: None,
            since,
            kinds: Vec::new(),
            wait: if since.is_some() {
                EVENTS_WAIT
            } else {
                Duration::ZERO
            },
        };
        match service.ask_waiting_if_running(&request, EVENTS_WAIT) {
            Ok(Asked::Answered(ServiceResponse::Events(page))) => {
                let resumed = since.replace(*page.cursor()).is_some();
                let reading = if resumed {
                    Some(
                        EventsReading::builder()
                            .events(page.events().clone())
                            .gap(*page.gap())
                            .build(),
                    )
                } else {
                    mem::take(&mut lost).then(|| {
                        EventsReading::builder()
                            .events(Vec::new())
                            .gap(true)
                            .build()
                    })
                };
                if let Some(reading) = reading
                    && !hub.send(HubEvent::Events(reading))
                {
                    return;
                }
                if !pause.wait(Duration::ZERO) {
                    return;
                }
            },
            _ => {
                lost |= since.take().is_some();
                if !pause.wait(EVENTS_RETRY) {
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        thread,
    };

    use crossbeam_channel::{bounded, unbounded};
    use muster_client::fake_service::FakeService;
    use muster_os::test_wait::wait_until;
    use muster_protocol::service::{EventKind, EventPage, EventSeq};

    use super::*;
    use crate::floor::fixture::stack_event;

    /// Longest the test waits for a page to reach the hub.
    const BOUND: Duration = Duration::from_secs(10);
    /// How long a parked poller is watched to stay parked.
    const PARKED_FOR: Duration = Duration::from_millis(300);

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

    /// The cursor each events read asked of `service` resumed from, and how
    /// long it asked the service to hold it.
    fn cursors(asked: &[ServiceRequest]) -> Vec<(Option<EventSeq>, Duration)> {
        asked
            .iter()
            .map(|request| match request {
                ServiceRequest::Events { since, wait, .. } => (*since, *wait),
                other => panic!("not an events read: {other:?}"),
            })
            .collect()
    }

    /// The reading `sent` carries.
    fn reading(sent: HubEvent) -> EventsReading {
        let HubEvent::Events(reading) = sent else {
            panic!("not an events page");
        };
        reading
    }

    /// The long poll resumes from each page's cursor, sends only pages read
    /// after one, and asks a read from no cursor not to wait, so a service
    /// holding it never answers it with events it would drop. Once a read
    /// finds no service it starts over from no cursor, and tells of a gap
    /// once it has one again.
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

        let first = reading(events.recv_timeout(BOUND).unwrap());
        let lost = reading(events.recv_timeout(BOUND).unwrap());
        let resumed = reading(events.recv_timeout(BOUND).unwrap());
        drop(events);
        poller.join().unwrap();

        for page in [&first, &resumed] {
            assert_eq!(page.events().len(), 1);
            assert!(!page.gap());
        }
        assert!(lost.events().is_empty());
        assert!(lost.gap());
        let asked = cursors(&service.asked());
        assert_eq!(asked[..5], [
            (None, Duration::ZERO),
            (Some(cursor(1)), EVENTS_WAIT),
            (Some(cursor(2)), EVENTS_WAIT),
            (None, Duration::ZERO),
            (Some(cursor(3)), EVENTS_WAIT),
        ]);
    }

    /// While no page streams the poller reads once, for its cursor, then
    /// parks asking nothing more; woken with a page streaming, it resumes
    /// from that cursor.
    #[test]
    fn the_long_poll_parks_while_no_page_streams() {
        let service = Arc::new(FakeService::default().asked_in_turn(vec![page(1), page(2)]));
        let (sender, events) = unbounded();
        let (wake, woken) = bounded(1);
        let watched = Arc::new(AtomicBool::new(false));
        let pause = Pause::parking(woken, Arc::clone(&watched));
        let hub = HubHandle::new(sender);
        let polled: Arc<dyn ServiceAccess> = service.clone();
        let poller = thread::spawn(move || poll_events(&polled, &hub, &pause));

        wait_until("the first read", || !service.asked().is_empty());
        thread::sleep(PARKED_FOR);
        let parked = service.asked().len();
        watched.store(true, Ordering::SeqCst);
        wake.send(()).unwrap();
        let resumed = reading(events.recv_timeout(BOUND).unwrap());
        drop((events, wake));
        poller.join().unwrap();

        assert_eq!(parked, 1);
        assert_eq!(resumed.events().len(), 1);
        assert_eq!(cursors(&service.asked())[1], (Some(cursor(1)), EVENTS_WAIT));
    }
}
