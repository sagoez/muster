use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    hub_event::HubEvent, hub_handle::HubHandle, pause::Pause, pauses::Pauses,
    subscribers::Subscribers,
};
use crate::floor::{Floor, Sequenced};

/// The hub thread's name.
const HUB_THREAD: &str = "view-hub";

/// The one owner of the floor: a loop that takes each event in turn, turns
/// it into the floor's messages and sends them to every open stream. No
/// other thread touches the floor, and the loop never waits on a stream.
/// While no page streams, the pollers that read only for the pages park.
pub struct Hub {
    /// The floor.
    floor: Floor,
    /// The open streams.
    subscribers: Subscribers,
    /// What every other thread sends.
    events: Receiver<HubEvent>,
    /// Whether any page streams, which the parking pollers wait on.
    watched: Arc<AtomicBool>,
    /// What wakes the status poller at once.
    wake_status: Sender<()>,
    /// The status poller's side of `wake_status`.
    status_woken: Receiver<()>,
    /// What wakes the notes poller from its park.
    wake_notes: Sender<()>,
    /// The notes poller's side of `wake_notes`.
    notes_woken: Receiver<()>,
    /// What wakes the logs poller from its park.
    wake_logs: Sender<()>,
    /// The logs poller's side of `wake_logs`.
    logs_woken: Receiver<()>,
    /// Never sent on: dropped with the hub, so every poller waiting on
    /// `lifeline` learns the hub is gone.
    _life: Sender<()>,
    /// The pollers' side of `_life`.
    lifeline: Receiver<()>,
}

impl Hub {
    /// A hub owning `floor`, and the handle that reaches it. No page
    /// streams yet, so the parking pollers park after their first read.
    pub fn new(floor: Floor) -> (Self, HubHandle) {
        let (sender, events) = unbounded();
        let (wake_status, status_woken) = bounded(1);
        let (wake_notes, notes_woken) = bounded(1);
        let (wake_logs, logs_woken) = bounded(1);
        let (life, lifeline) = bounded(0);
        let hub = Self {
            floor,
            subscribers: Subscribers::default(),
            events,
            watched: Arc::new(AtomicBool::new(false)),
            wake_status,
            status_woken,
            wake_notes,
            notes_woken,
            wake_logs,
            logs_woken,
            _life: life,
            lifeline,
        };
        (hub, HubHandle::new(sender))
    }

    /// How each poller waits on this hub.
    pub fn pauses(&self) -> Pauses {
        let parking =
            |woken: &Receiver<()>| Pause::parking(woken.clone(), Arc::clone(&self.watched));
        Pauses::builder()
            .life(Pause::new(self.lifeline.clone()))
            .status(parking(&self.status_woken))
            .notes(parking(&self.notes_woken))
            .logs(parking(&self.logs_woken))
            .build()
    }

    /// Runs the loop on a thread of its own until the view stops it.
    ///
    /// # Errors
    /// Returns the failure to start the thread.
    pub fn spawn(self) -> io::Result<JoinHandle<()>> {
        thread::Builder::new()
            .name(HUB_THREAD.to_string())
            .spawn(move || self.run())
    }

    /// Takes each event in turn and sends what it changed to every open
    /// stream, then looks whether any page still streams, until told to
    /// stop.
    fn run(mut self) {
        while let Ok(event) = self.events.recv() {
            let Some(messages) = self.take(event) else {
                return;
            };
            self.subscribers.broadcast(&messages);
            self.watch();
        }
    }

    /// Parks or wakes the pollers that read for the pages as the first page
    /// connects or the last one goes. A floor nobody watched missed what
    /// those pollers did not read, so the first page is sent a snapshot
    /// once fresh readings arrive.
    fn watch(&mut self) {
        let watched = self.subscribers.any_open();
        if self.watched.swap(watched, Ordering::SeqCst) == watched || !watched {
            return;
        }
        self.floor.gap();
        for wake in [&self.wake_status, &self.wake_notes, &self.wake_logs] {
            let _ = wake.try_send(());
        }
    }

    /// The messages `event` makes the floor send; none when it stops the
    /// hub. An ask is answered here; an answer nobody waits for any more is
    /// dropped. The first page to connect while none watched finds the
    /// floor rested, so its snapshot confirms nothing read while the
    /// pollers parked, until the reading they wake to.
    fn take(&mut self, event: HubEvent) -> Option<Vec<Sequenced>> {
        let messages = match event {
            HubEvent::Projects(reading) => self.floor.read_projects(reading),
            HubEvent::Asking => {
                self.floor.ask();
                Vec::new()
            },
            HubEvent::Status { reading, now, wall } => self.floor.read_status(reading, now, wall),
            HubEvent::Events(reading) => {
                self.floor.hear_events(&reading);
                if reading.gap() || !reading.events().is_empty() {
                    let _ = self.wake_status.try_send(());
                }
                Vec::new()
            },
            HubEvent::Notes { root, notes } => self.floor.read_notes(&root, &notes),
            HubEvent::Logs { now, readings } => self.floor.read_logs(now, &readings),
            HubEvent::Subscribe(reply) => {
                let subscription = if self.subscribers.full() {
                    None
                } else {
                    if !self.watched.load(Ordering::SeqCst) {
                        self.floor.rest();
                    }
                    let snapshot = self.floor.snapshot();
                    self.subscribers.admit(&snapshot)
                };
                let _ = reply.send(subscription);
                Vec::new()
            },
            HubEvent::LocateLog {
                project,
                name,
                reply,
            } => {
                let _ = reply.send(self.floor.locate_log(&project, &name));
                Vec::new()
            },
            HubEvent::LogWatches(reply) => {
                let _ = reply.send(self.floor.log_watches());
                Vec::new()
            },
            HubEvent::Stop => return None,
        };
        Some(messages)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant, SystemTime},
    };

    use muster_domain::{
        coordination::{Author, Scratchpad, ScratchpadKey},
        process::{ProcessState, Readiness},
        value::EpochSeconds,
    };
    use muster_protocol::service::EventKind;

    use super::*;
    use crate::{
        floor::{
            EventsReading, ProjectsReading, StateTrouble, StatusReading,
            fixture::{floor, process_status, project_reading, project_status, root, stack_event},
        },
        http::{EVENT_FIELD, ID_FIELD},
        live::{
            subscribers::{MAX_STREAMS, STREAM_BUFFER},
            subscription::Subscription,
        },
    };

    /// The project the hub's floor holds.
    const SHOP: &str = "shop";
    /// Longest a test waits for a frame or a wake.
    const BOUND: Duration = Duration::from_secs(10);
    /// The period a test's poller waits between reads.
    const SHORT: Duration = Duration::from_millis(10);
    /// How long a parked poller is watched to stay parked.
    const PARKED_FOR: Duration = Duration::from_millis(300);
    /// Changes the stalled-stream test makes: more than a stream holds
    /// behind its snapshot.
    const CHANGES: usize = STREAM_BUFFER + 2;

    /// A started hub over the fixture floor holding shop's `web`, its
    /// handle, what it wakes the status poller through, and its thread.
    fn started() -> (HubHandle, Receiver<()>, JoinHandle<()>) {
        let (hub, handle) = Hub::new(floor());
        let woken = hub.status_woken.clone();
        let thread = hub.spawn().unwrap();
        assert!(handle.send(HubEvent::Projects(ProjectsReading::Read(vec![
            project_reading(SHOP, &["web"])
        ]))));
        (handle, woken, thread)
    }

    /// Shop's `web` reported in `state`, asked for and sent to the hub as
    /// the status poller does.
    fn report(handle: &HubHandle, state: ProcessState) {
        assert!(handle.send(HubEvent::Asking));
        assert!(handle.send(HubEvent::Status {
            reading: StatusReading::Answered(vec![project_status(SHOP, vec![process_status(
                "web",
                state,
                Readiness::None,
            )])]),
            now: Instant::now(),
            wall: SystemTime::now(),
        }));
    }

    /// The event a frame names.
    fn kind_of(frame: &str) -> &str {
        frame
            .lines()
            .next()
            .and_then(|line| line.strip_prefix(EVENT_FIELD))
            .unwrap()
    }

    /// The sequence number a frame carries.
    fn seq_of(frame: &str) -> u64 {
        frame
            .lines()
            .nth(1)
            .and_then(|line| line.strip_prefix(ID_FIELD))
            .unwrap()
            .parse()
            .unwrap()
    }

    /// The next frame of `subscription`, failing past [`BOUND`].
    fn next(subscription: &Subscription) -> Arc<str> {
        subscription.frames().recv_timeout(BOUND).unwrap()
    }

    /// A note of shop's whose body is `body`.
    fn note(body: String) -> Scratchpad {
        Scratchpad::builder()
            .key(ScratchpadKey::try_new("plan").unwrap())
            .body(body)
            .author(Author::Human)
            .updated_at(EpochSeconds::new(1))
            .build()
    }

    /// Waits until the hub has taken everything sent before, by asking it
    /// something.
    fn settle(handle: &HubHandle) {
        handle.log_watches().unwrap();
    }

    /// Waits on `pause` for [`SHORT`] on a thread of its own; what the wait
    /// returned arrives on the receiver once it ends.
    fn waiting(pause: &Pause) -> Receiver<bool> {
        let (ended, wait) = bounded(1);
        let pause = pause.clone();
        thread::spawn(move || {
            let _ = ended.send(pause.wait(SHORT));
        });
        wait
    }

    /// A new stream gets the snapshot first, naming the floor's processes.
    #[test]
    fn a_new_subscriber_gets_a_snapshot_first() {
        let (handle, _woken, _thread) = started();
        report(&handle, ProcessState::Running);

        let subscription = handle.subscribe().unwrap().unwrap();

        let frame = next(&subscription);
        assert_eq!(kind_of(&frame), "snapshot");
        assert!(frame.contains("\"name\":\"web\""), "{frame}");
    }

    /// A stream that stops reading is dropped once its buffer is full, and
    /// the stream that reads gets every message in order.
    #[test]
    fn a_stalled_subscriber_is_dropped_and_others_keep_receiving() {
        let (handle, _woken, _thread) = started();
        let stalled = handle.subscribe().unwrap().unwrap();
        let reading = handle.subscribe().unwrap().unwrap();

        let mut read = vec![next(&reading)];
        for change in 0..CHANGES {
            assert!(handle.send(HubEvent::Notes {
                root: root(SHOP),
                notes: vec![note(change.to_string())],
            }));
            read.push(next(&reading));
        }
        settle(&handle);

        assert_eq!(read.len(), CHANGES + 1);
        assert_eq!(kind_of(&read[0]), "snapshot");
        assert!(read[1..].iter().all(|frame| kind_of(frame) == "notes"));
        assert!(
            read.windows(2)
                .all(|pair| seq_of(&pair[1]) > seq_of(&pair[0]))
        );
        let held: Vec<Arc<str>> = stalled.frames().try_iter().collect();
        assert_eq!(held.len(), STREAM_BUFFER);
        assert!(stalled.frames().recv_timeout(BOUND).is_err());
    }

    /// An event page with events wakes the status poller at once and its
    /// answer goes out as changes; one with a gap wakes it too, and its
    /// answer goes out as a snapshot; one with neither wakes nothing.
    #[test]
    fn an_event_wakes_a_poll_and_a_gap_a_snapshot() {
        let (handle, woken, _thread) = started();
        let subscription = handle.subscribe().unwrap().unwrap();
        assert_eq!(kind_of(&next(&subscription)), "snapshot");
        woken.recv_timeout(BOUND).unwrap();
        report(&handle, ProcessState::Running);
        assert_eq!(kind_of(&next(&subscription)), "snapshot");

        assert!(
            handle.send(HubEvent::Events(
                EventsReading::builder()
                    .events(Vec::new())
                    .gap(false)
                    .build()
            ))
        );
        settle(&handle);
        assert!(woken.try_recv().is_err());

        assert!(
            handle.send(HubEvent::Events(
                EventsReading::builder()
                    .events(vec![stack_event(SHOP, "web", EventKind::Crashed)])
                    .gap(false)
                    .build()
            ))
        );
        woken.recv_timeout(BOUND).unwrap();
        report(&handle, ProcessState::Crashed);
        assert_eq!(kind_of(&next(&subscription)), "process");
        assert_eq!(kind_of(&next(&subscription)), "pin");

        assert!(
            handle.send(HubEvent::Events(
                EventsReading::builder()
                    .events(Vec::new())
                    .gap(true)
                    .build()
            ))
        );
        woken.recv_timeout(BOUND).unwrap();
        report(&handle, ProcessState::Crashed);
        assert_eq!(kind_of(&next(&subscription)), "snapshot");
    }

    /// Past the most streams one more is refused, and a closed one frees
    /// its place.
    #[test]
    fn streams_past_the_cap_are_refused() {
        let (handle, _woken, _thread) = started();
        let mut open: Vec<Subscription> = (0..MAX_STREAMS)
            .map(|_| handle.subscribe().unwrap().unwrap())
            .collect();

        assert!(handle.subscribe().unwrap().is_none());
        open.pop();
        assert!(handle.subscribe().unwrap().is_some());
    }

    /// While no page streams, the status, notes and logs pollers park and
    /// the projects' does not; the first page wakes them, they park again
    /// once it goes, and a parked one learns when the hub is gone.
    #[test]
    fn the_pollers_for_the_pages_park_while_none_streams() {
        let (hub, handle) = Hub::new(floor());
        let pauses = hub.pauses();
        let thread = hub.spawn().unwrap();

        assert!(pauses.life().wait(SHORT));
        let parked: Vec<Receiver<bool>> = [pauses.status(), pauses.notes(), pauses.logs()]
            .into_iter()
            .map(waiting)
            .collect();
        for wait in &parked {
            assert!(wait.recv_timeout(PARKED_FOR).is_err(), "it did not park");
        }
        let subscription = handle.subscribe().unwrap().unwrap();
        for wait in &parked {
            assert_eq!(wait.recv_timeout(BOUND), Ok(true));
        }
        drop(subscription);
        // The hub looks for closed streams after each event it takes, so the
        // second ask is answered only once it has seen the first page go.
        settle(&handle);
        settle(&handle);
        let again = waiting(pauses.notes());

        assert!(
            again.recv_timeout(PARKED_FOR).is_err(),
            "it did not park again"
        );
        handle.stop();
        thread.join().unwrap();
        assert_eq!(again.recv_timeout(BOUND), Ok(false));
    }

    /// The first page after none streamed, at start or once the pollers
    /// parked again, is sent the floor as last read, loading and with every
    /// process unconfirmed, then a snapshot once a fresh status arrives,
    /// though nothing changed since the last: the floor missed whatever
    /// happened while nobody watched.
    #[test]
    fn the_first_page_gets_a_snapshot_of_fresh_readings() {
        let (handle, woken, _thread) = started();
        report(&handle, ProcessState::Running);
        settle(&handle);

        for page in 0..2 {
            let subscription = handle.subscribe().unwrap().unwrap();
            let rested = next(&subscription);
            woken.recv_timeout(BOUND).unwrap();
            report(&handle, ProcessState::Running);
            let fresh = next(&subscription);
            drop(subscription);
            // The hub looks for closed streams after each event it takes.
            settle(&handle);
            settle(&handle);

            assert_eq!(kind_of(&rested), "snapshot", "page {page}");
            assert!(rested.contains("\"health\":\"loading\""), "{rested}");
            assert!(rested.contains("\"unconfirmed\":\"resting\""), "{rested}");
            assert_eq!(kind_of(&fresh), "snapshot", "page {page}");
            assert!(fresh.contains("\"health\":\"running\""), "{fresh}");
            assert!(fresh.contains("\"unconfirmed\":null"), "{fresh}");
        }
    }

    /// A stop ends the hub: its streams end, its pollers learn it is gone,
    /// and an ask fails.
    #[test]
    fn a_stop_ends_the_hub_its_streams_and_its_pollers() {
        let (hub, handle) = Hub::new(floor());
        let pauses = hub.pauses();
        let thread = hub.spawn().unwrap();
        let subscription = handle.subscribe().unwrap().unwrap();
        // A woken pause may first take the wake the page left it.
        let ended = |pause: &Pause| (0..2).any(|_| !pause.wait(BOUND));

        handle.stop();
        thread.join().unwrap();

        assert_eq!(kind_of(&next(&subscription)), "snapshot");
        assert!(subscription.frames().recv_timeout(BOUND).is_err());
        assert!(!pauses.life().wait(BOUND));
        for pause in [pauses.status(), pauses.notes(), pauses.logs()] {
            assert!(ended(pause));
        }
        assert!(handle.subscribe().is_err());
        assert!(!handle.send(HubEvent::Projects(ProjectsReading::Trouble(
            StateTrouble::NoState
        ))));
    }
}
