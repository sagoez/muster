use std::{
    io,
    thread::{self, JoinHandle},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause, subscribers::Subscribers};
use crate::floor::{Floor, Sequenced};

/// The hub thread's name.
const HUB_THREAD: &str = "view-hub";

/// The one owner of the floor: a loop that takes each event in turn, turns
/// it into the floor's messages and sends them to every open stream. No
/// other thread touches the floor, and the loop never waits on a stream.
pub struct Hub {
    /// The floor.
    floor: Floor,
    /// The open streams.
    subscribers: Subscribers,
    /// What every other thread sends.
    events: Receiver<HubEvent>,
    /// What wakes the status poller at once.
    wake: Sender<()>,
    /// The status poller's side of `wake`.
    woken: Receiver<()>,
    /// Never sent on: dropped with the hub, so every poller waiting on
    /// `lifeline` learns the hub is gone.
    _life: Sender<()>,
    /// The pollers' side of `_life`.
    lifeline: Receiver<()>,
}

impl Hub {
    /// A hub owning `floor`, and the handle that reaches it.
    pub fn new(floor: Floor) -> (Self, HubHandle) {
        let (sender, events) = unbounded();
        let (wake, woken) = bounded(1);
        let (life, lifeline) = bounded(0);
        let hub = Self {
            floor,
            subscribers: Subscribers::default(),
            events,
            wake,
            woken,
            _life: life,
            lifeline,
        };
        (hub, HubHandle::new(sender))
    }

    /// How the status poller waits: woken at once when an event page asks
    /// for a fresh status.
    pub fn wake(&self) -> Pause {
        Pause::new(self.woken.clone())
    }

    /// How every other poller waits, until the hub is gone.
    pub fn life(&self) -> Pause {
        Pause::new(self.lifeline.clone())
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
    /// stream, until told to stop.
    fn run(mut self) {
        while let Ok(event) = self.events.recv() {
            let Some(messages) = self.take(event) else {
                return;
            };
            self.subscribers.broadcast(&messages);
        }
    }

    /// The messages `event` makes the floor send; none when it stops the
    /// hub. An ask is answered here; an answer nobody waits for any more is
    /// dropped.
    fn take(&mut self, event: HubEvent) -> Option<Vec<Sequenced>> {
        let messages = match event {
            HubEvent::Projects(reading) => self.floor.projects(reading),
            HubEvent::Status { reading, now, wall } => self.floor.status(reading, now, wall),
            HubEvent::Events(reading) => {
                self.floor.events(&reading);
                if reading.gap() || !reading.events().is_empty() {
                    let _ = self.wake.try_send(());
                }
                Vec::new()
            },
            HubEvent::Notes { root, notes } => self.floor.notes(&root, &notes),
            HubEvent::Logs { now, readings } => self.floor.logs(now, &readings),
            HubEvent::Subscribe(reply) => {
                let subscription = if self.subscribers.full() {
                    None
                } else {
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
            EventsReading, ProjectsReading, StatusReading,
            fixture::{floor, process_status, project_reading, project_status, root, stack_event},
        },
        live::{
            subscribers::{MAX_STREAMS, STREAM_BUFFER},
            subscription::Subscription,
        },
    };

    /// The project the hub's floor holds.
    const SHOP: &str = "shop";
    /// Longest a test waits for a frame or a wake.
    const BOUND: Duration = Duration::from_secs(10);
    /// Changes the stalled-stream test makes: more than a stream holds
    /// behind its snapshot.
    const CHANGES: usize = STREAM_BUFFER + 2;

    /// A started hub over the fixture floor holding shop's `web`, its
    /// handle, what it wakes the status poller through, and its thread.
    fn started() -> (HubHandle, Receiver<()>, JoinHandle<()>) {
        let (hub, handle) = Hub::new(floor());
        let woken = hub.woken.clone();
        let thread = hub.spawn().unwrap();
        assert!(handle.send(HubEvent::Projects(ProjectsReading::Read(vec![
            project_reading(SHOP, &["web"])
        ]))));
        (handle, woken, thread)
    }

    /// Shop's `web` reported in `state`, sent to the hub.
    fn report(handle: &HubHandle, state: ProcessState) {
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
            .and_then(|line| line.strip_prefix("event: "))
            .unwrap()
    }

    /// The sequence number a frame carries.
    fn seq_of(frame: &str) -> u64 {
        frame
            .lines()
            .nth(1)
            .and_then(|line| line.strip_prefix("id: "))
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

    /// A new stream gets the snapshot first, naming the floor's processes.
    #[test]
    fn a_new_subscriber_gets_a_snapshot_first() {
        let (handle, _woken, _thread) = started();

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
        report(&handle, ProcessState::Running);
        let subscription = handle.subscribe().unwrap().unwrap();
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

    /// A stop ends the hub: its streams end, its pollers learn it is gone,
    /// and an ask fails.
    #[test]
    fn a_stop_ends_the_hub_its_streams_and_its_pollers() {
        let (hub, handle) = Hub::new(floor());
        let (life, wake) = (hub.life(), hub.wake());
        let thread = hub.spawn().unwrap();
        let subscription = handle.subscribe().unwrap().unwrap();

        handle.stop();
        thread.join().unwrap();

        assert_eq!(kind_of(&next(&subscription)), "snapshot");
        assert!(subscription.frames().recv_timeout(BOUND).is_err());
        assert!(!life.wait(BOUND));
        assert!(!wake.wait(BOUND));
        assert!(handle.subscribe().is_err());
        assert!(!handle.send(HubEvent::Projects(ProjectsReading::NoState)));
    }
}
