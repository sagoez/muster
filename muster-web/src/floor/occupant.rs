use std::{
    mem,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use getset::Getters;
use muster_client::listening_ports;
use muster_domain::{
    process::{ProcessIcon, ProcessState},
    value::{ProcessName, ProjectName},
};
use muster_protocol::service::{EventKind, ProcessStatus};

use super::{
    message::Pin,
    reading::{LogStat, StackedProcess},
    view::{EndedView, FaceView, Mark, NextStep, Placement},
};

/// How far a fresh reckoning of when a child started may stray from the
/// kept one and still be the same start: uptime is whole seconds read a
/// moment after it was sampled, so each poll reckons a little differently.
const START_SLACK: Duration = Duration::from_secs(2);
/// Most events one process keeps heard until a status reading resolves
/// them, so a flapping process the service cannot be asked about holds
/// only its latest few.
const MAX_HEARD: usize = 8;
/// Least time between two messages that change only a process's usage.
pub(super) const USAGE_EVERY: Duration = Duration::from_secs(5);
/// The heard events that mean a process left its room since the last poll,
/// so a pin it earns after them is a new one even when it equals the last.
const LEAVING_KINDS: [EventKind; 2] = [EventKind::Restarting, EventKind::Started];

/// One process on the floor and what the floor keeps of it between readings.
#[derive(Getters)]
pub(super) struct Occupant {
    /// Its name within its project.
    #[getset(get = "pub(super)")]
    name: ProcessName,
    /// The icon it wears.
    #[getset(get = "pub(super)")]
    icon: ProcessIcon,
    /// What the service last reported of it, none while its project is not
    /// loaded or no service has answered.
    status: Option<ProcessStatus>,
    /// When its child started, in seconds since the Unix epoch, kept steady
    /// across polls.
    started: Option<u64>,
    /// Whether a status reading has placed it yet; until then a change of
    /// its room is no change anyone saw, so it earns no pin.
    seen: bool,
    /// The events heard of it that a poll may have missed and no status
    /// reading has taken yet, each with when it happened, oldest first.
    heard: Vec<(EventKind, SystemTime)>,
    /// The last pin it earned while it stays in the same room, so one change
    /// heard both as an event and in a poll pins once.
    last_pin: Option<Pin>,
    /// The face every page was last sent of it, none before the first.
    shown: Option<FaceView>,
    /// When a message last carried its usage.
    usage_sent: Option<Instant>,
    /// Its log's last stat: none before the first, `Some(None)` while it had
    /// no log.
    log: Option<Option<LogStat>>,
    /// Until when it speaks, while it does.
    speaking_until: Option<Instant>,
}

impl Occupant {
    /// The process `process` of a stack, seen for the first time.
    pub(super) fn new(process: &StackedProcess) -> Self {
        Self {
            name: process.name().clone(),
            icon: *process.icon(),
            status: None,
            started: None,
            seen: false,
            heard: Vec::new(),
            last_pin: None,
            shown: None,
            usage_sent: None,
            log: None,
            speaking_until: None,
        }
    }

    /// The same process as `process` of a stack now names it and marks it,
    /// keeping all else the floor knew of it.
    pub(super) fn renamed(self, process: &StackedProcess) -> Self {
        Self {
            name: process.name().clone(),
            icon: *process.icon(),
            ..self
        }
    }

    /// Whether its child runs, so its log is worth statting.
    pub(super) fn runs(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| *status.state() == ProcessState::Running)
    }

    /// Whether a status reading has placed it yet.
    pub(super) fn seen(&self) -> bool {
        self.seen
    }

    /// Records that a status reading placed it; says whether one had before.
    pub(super) fn see(&mut self) -> bool {
        mem::replace(&mut self.seen, true)
    }

    /// Keeps an event of `kind` heard of it, which happened `at`, dropping
    /// the oldest past [`MAX_HEARD`].
    pub(super) fn hear(&mut self, kind: EventKind, at: SystemTime) {
        if self.heard.len() == MAX_HEARD {
            self.heard.remove(0);
        }
        self.heard.push((kind, at));
    }

    /// The kinds heard of it that a status reading asked for at `wall` takes
    /// in: those that happened from `since`, when the reading before was
    /// asked for, up to `wall`. An older one that reading already showed is
    /// forgotten; a newer one happened after the ask, so it waits for the
    /// next reading.
    pub(super) fn take_heard(
        &mut self,
        since: Option<SystemTime>,
        wall: SystemTime,
    ) -> Vec<EventKind> {
        let (later, taken): (Vec<_>, Vec<_>) = mem::take(&mut self.heard)
            .into_iter()
            .partition(|(_, at)| *at > wall);
        self.heard = later;
        taken
            .into_iter()
            .filter(|(_, at)| since.is_none_or(|since| *at >= since))
            .map(|(kind, _)| kind)
            .collect()
    }

    /// Takes in `status`, read at `wall`: its child's start is reckoned
    /// from its uptime, keeping the start already known when the two are
    /// within [`START_SLACK`]. Returns the status it held before.
    pub(super) fn place(
        &mut self,
        status: Option<ProcessStatus>,
        wall: SystemTime,
    ) -> Option<ProcessStatus> {
        let reckoned = status
            .as_ref()
            .and_then(|status| *status.uptime())
            .and_then(|uptime| wall.checked_sub(uptime))
            .and_then(|start| start.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs());
        self.started = match (self.started, reckoned) {
            (Some(kept), Some(now)) if kept.abs_diff(now) <= START_SLACK.as_secs() => Some(kept),
            (_, now) => now,
        };
        mem::replace(&mut self.status, status)
    }

    /// Takes the pin of the status it holds as the last it earned, so a
    /// baseline or a first sight of it pins nothing that status shows.
    pub(super) fn settle_pin(&mut self) {
        self.last_pin = Pin::between(None, self.status.as_ref());
    }

    /// The pins it earns going from `was` to the status it now holds,
    /// having heard `heard` since: each heard event's pin, then the pin of
    /// its change of room. A change of room forgets the last pin, and so
    /// does a heard start or restart, since the process left its room and
    /// came back between polls; within a room a pin equal to the last one
    /// is not earned again, so a change both heard and polled pins once.
    pub(super) fn earn_pins(
        &mut self,
        was: Option<&ProcessStatus>,
        heard: &[EventKind],
    ) -> Vec<Pin> {
        let now = self.status.as_ref();
        if Placement::of(was).room() != Placement::of(now).room() {
            self.last_pin = None;
        }
        let mut earned = Vec::new();
        let mut earn = |last_pin: &mut Option<Pin>, pin: Pin| {
            if last_pin.replace(pin) != Some(pin) {
                earned.push(pin);
            }
        };
        for kind in heard {
            if LEAVING_KINDS.contains(kind) {
                self.last_pin = None;
            }
            if let Some(pin) = Pin::heard(*kind, now) {
                earn(&mut self.last_pin, pin);
            }
        }
        if let Some(pin) = Pin::between(was, now) {
            earn(&mut self.last_pin, pin);
        }
        earned
    }

    /// Takes in its log's stat, `stat`: when the log changed since the last
    /// stat and its child runs, it speaks until `until`. Says whether it
    /// started speaking.
    pub(super) fn stat_log(&mut self, stat: Option<LogStat>, until: Instant) -> bool {
        let before = self.log.replace(stat);
        let printed = before.is_some_and(|before| before != stat);
        printed && self.runs() && self.speaking_until.replace(until).is_none()
    }

    /// Falls quiet when its time to speak is up at `now`; says whether it
    /// did.
    pub(super) fn quiet_if_due(&mut self, now: Instant) -> bool {
        if self.speaking_until.is_some_and(|until| until <= now) {
            self.speaking_until = None;
            return true;
        }
        false
    }

    /// Forgets its log's stat and falls quiet, as when nobody watches the
    /// logs, so the first stat after takes the log as it finds it.
    pub(super) fn forget_log(&mut self) {
        self.log = None;
        self.speaking_until = None;
    }

    /// Whether `face` is worth a message at `now`: it differs from the face
    /// last shown beyond its usage, or its usage changed and none was sent
    /// in the last [`USAGE_EVERY`].
    pub(super) fn face_due(&self, face: &FaceView, now: Instant) -> bool {
        let Some(shown) = &self.shown else {
            return true;
        };
        let usage_due = self
            .usage_sent
            .is_none_or(|sent| now.saturating_duration_since(sent) >= USAGE_EVERY);
        shown.differs_beyond_usage(face) || (shown.usage_differs(face) && usage_due)
    }

    /// Records that every page was sent `face`, its usage at `now` when
    /// known.
    pub(super) fn show(&mut self, face: FaceView, now: Option<Instant>) {
        self.shown = Some(face);
        if now.is_some() {
            self.usage_sent = now;
        }
    }

    /// The process as the page draws it, its card's command naming
    /// `project`. A status no service has `confirmed` since marks it muted,
    /// though it keeps its room and its command. A CPU use that is no
    /// number reads as unknown.
    pub(super) fn face(&self, project: &ProjectName, confirmed: bool) -> FaceView {
        let status = self.status.as_ref();
        let placement = Placement::of(status);
        let state = status.map(|status| *status.state()).unwrap_or_default();
        let usage = status.and_then(|status| status.usage().as_ref());
        let mark = if status.is_some() && !confirmed {
            Some(Mark::Muted)
        } else {
            placement.mark()
        };
        FaceView::builder()
            .name(self.name.clone())
            .next(NextStep::of(project, &self.name, placement, state))
            .icon(self.icon)
            .room(placement.room())
            .mark(mark)
            .state(state)
            .ports(status.map(listening_ports).unwrap_or_default())
            .started(self.started)
            .cpu(
                usage
                    .map(|usage| *usage.cpu_percent())
                    .filter(|cpu| cpu.is_finite()),
            )
            .memory(usage.map(|usage| *usage.memory_bytes()))
            .restarts(status.map(|status| *status.restarts()).unwrap_or_default())
            .ended(status.and_then(|status| status.last_exit().as_ref().map(EndedView::from)))
            .attached(status.map(|status| *status.attached()).unwrap_or_default())
            .speaking(self.speaking_until.is_some())
            .build()
    }
}
