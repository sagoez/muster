use std::{
    mem,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use muster_client::listening_ports;
use muster_domain::{
    process::{ProcessIcon, ProcessState},
    value::{ProcessName, ProjectName},
};
use muster_protocol::service::{EventKind, ProcessStatus};

use super::{
    message::Pin,
    reading::{LogStat, StackedProcess},
    view::{EndedView, FaceView, NextStep, Placement},
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

/// One process on the floor and what the floor keeps of it between readings.
pub(super) struct Occupant {
    /// Its name within its project.
    pub(super) name: ProcessName,
    /// The icon it wears.
    pub(super) icon: ProcessIcon,
    /// What the service last reported of it, none while its project is not
    /// loaded or no service answers.
    pub(super) status: Option<ProcessStatus>,
    /// When its child started, in seconds since the Unix epoch, kept steady
    /// across polls.
    pub(super) started: Option<u64>,
    /// Whether a status reading has placed it yet; until then a change of
    /// its room is no change anyone saw, so it earns no pin.
    pub(super) seen: bool,
    /// The events heard of it since the last status reading that a poll may
    /// have missed, each with when it happened, oldest first.
    pub(super) heard: Vec<(EventKind, SystemTime)>,
    /// The last pin it earned while it stays in the same room, so one change
    /// heard both as an event and in a poll pins once.
    pub(super) last_pin: Option<Pin>,
    /// The face every page was last sent of it, none before the first.
    pub(super) shown: Option<FaceView>,
    /// When a message last carried its usage.
    pub(super) usage_sent: Option<Instant>,
    /// Its log's last stat: none before the first, `Some(None)` while it had
    /// no log.
    pub(super) log: Option<Option<LogStat>>,
    /// Until when it speaks, while it does.
    pub(super) speaking_until: Option<Instant>,
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

    /// Whether its child runs, so its log is worth statting.
    pub(super) fn runs(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| *status.state() == ProcessState::Running)
    }

    /// Keeps an event of `kind` heard of it, which happened `at`, dropping
    /// the oldest past [`MAX_HEARD`].
    pub(super) fn hear(&mut self, kind: EventKind, at: SystemTime) {
        if self.heard.len() == MAX_HEARD {
            self.heard.remove(0);
        }
        self.heard.push((kind, at));
    }

    /// The kinds heard of it that happened no earlier than `since`, the
    /// status reading before the one taking them in, which already showed
    /// anything older; all of them when there was none. Forgets them all.
    pub(super) fn take_heard(&mut self, since: Option<SystemTime>) -> Vec<EventKind> {
        mem::take(&mut self.heard)
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
    /// `project`. A CPU use that is no number reads as unknown.
    pub(super) fn face(&self, project: &ProjectName) -> FaceView {
        let status = self.status.as_ref();
        let placement = Placement::of(status);
        let state = status.map(|status| *status.state()).unwrap_or_default();
        let usage = status.and_then(|status| status.usage().as_ref());
        FaceView::builder()
            .name(self.name.clone())
            .next(NextStep::of(project, &self.name, placement, state))
            .icon(self.icon)
            .room(placement.room())
            .mark(placement.mark())
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
