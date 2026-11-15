use std::{
    mem,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use getset::{CopyGetters, Getters};
use muster_client::listening_ports;
use muster_domain::{
    process::{ProcessIcon, ProcessState},
    value::ProcessName,
};
use muster_protocol::service::ProcessStatus;
use typed_builder::TypedBuilder;

use super::{
    reading::{LogStat, StackedProcess},
    view::{EndedView, FaceView, NextStep, Placement, Unconfirmed},
};

/// How far a fresh reckoning of when a child started may stray from the
/// kept one and still be the same start: uptime is whole seconds read a
/// moment after it was sampled, so each poll reckons a little differently.
const START_SLACK: Duration = Duration::from_secs(2);
/// Least time between two messages that change only a process's usage.
pub(super) const USAGE_EVERY: Duration = Duration::from_secs(5);

/// One process on the floor and what the floor keeps of it between readings.
/// Its builder takes only its name and icon: all it keeps between readings
/// starts empty, as for a process no reading has placed yet.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub(super) struct Occupant {
    /// Its name within its project.
    #[getset(get = "pub(super)")]
    name: ProcessName,
    /// The icon it wears.
    #[getset(get = "pub(super)")]
    icon: ProcessIcon,
    /// What the service last reported of it, none while its project is not
    /// loaded or no service has answered.
    #[builder(default, setter(skip))]
    status: Option<ProcessStatus>,
    /// When its child started, in seconds since the Unix epoch, kept steady
    /// across polls.
    #[builder(default, setter(skip))]
    started: Option<u64>,
    /// Whether a status reading has placed it yet; until then a change of
    /// its status is no change anyone saw, so it floats nothing.
    #[getset(get_copy = "pub(super)")]
    #[builder(default, setter(skip))]
    seen: bool,
    /// The face every page was last sent of it, none before the first.
    #[builder(default, setter(skip))]
    shown: Option<FaceView>,
    /// When a message last carried its usage.
    #[builder(default, setter(skip))]
    usage_sent: Option<Instant>,
    /// Its log's last stat: none before the first, `Some(None)` while it had
    /// no log.
    #[builder(default, setter(skip))]
    log: Option<Option<LogStat>>,
    /// Until when it speaks, while it does.
    #[builder(default, setter(skip))]
    speaking_until: Option<Instant>,
}

impl Occupant {
    /// The process `process` of a stack, seen for the first time.
    pub(super) fn new(process: &StackedProcess) -> Self {
        Self::builder()
            .name(process.name().clone())
            .icon(*process.icon())
            .build()
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

    /// Records that a status reading placed it; says whether one had before.
    pub(super) fn see(&mut self) -> bool {
        mem::replace(&mut self.seen, true)
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

    /// The process as the page draws it, its card's command naming its
    /// project with the `-p` words `project`, none when there are none.
    /// While its status is `unconfirmed` it is drawn so, saying why and
    /// keeping its room, its mark and its command. A CPU use that is no
    /// number reads as unknown.
    pub(super) fn face(&self, project: Option<&str>, unconfirmed: Option<Unconfirmed>) -> FaceView {
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
            .unconfirmed(unconfirmed)
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
