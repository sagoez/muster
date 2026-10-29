use std::time::{Duration, Instant};

use rustix::process::Signal;

use super::{
    config_membership::ConfigMembership, entity::Supervisor, unsupervised_child::UnsupervisedChild,
};
use crate::{
    adapter::{
        pty::{signal_group, unix_signal},
        service::{NoticeLevel, Refusal},
    },
    domain::{
        process::{ExitIntent, ProcessState},
        value::{PaneId, Pid, StartToken},
    },
};

/// Interval between checks that a signalled unsupervised process is gone.
const UNSUPERVISED_EXIT_POLL: Duration = Duration::from_millis(100);
/// Most kills sent to an unsupervised process before the stop gives up and
/// leaves it for another try: five seconds of polling.
const UNSUPERVISED_KILL_LIMIT: u32 = 50;
/// Prefix for a launch refused because the process still runs unsupervised;
/// the pid follows.
const UNSUPERVISED_START_REFUSED: &str =
    "cannot start: the process was left running unsupervised as pid";
/// Shown when a start finds the process gone and its row, already removed
/// from the project, retired with it, so there is nothing left to start.
pub(super) const UNSUPERVISED_ROW_REMOVED: &str =
    "cannot start: the process was removed from the project and has exited";
/// Prefix for an unsupervised process the stop could not signal; the pid
/// follows.
const UNSUPERVISED_STOP_FAILED: &str =
    "could not signal the process left running unsupervised as pid";
/// Prefix for an unsupervised process still alive after every kill; the pid
/// follows.
const UNSUPERVISED_KILL_GAVE_UP: &str =
    "the process left running unsupervised survived every kill; stop it again to retry. pid";

impl Supervisor {
    /// Records that `pane`'s process was left running under a holder this
    /// service cannot reach, as the child `pid`, when that child runs. The pane
    /// keeps a terminal but no handle, refuses to start, and a stop signals the
    /// child directly. A child recorded with a start token is only taken while
    /// the pid still carries that token, so a reused pid is never mistaken for
    /// it. Returns whether the child runs.
    pub(super) fn leave_unsupervised(
        &mut self,
        pane: PaneId,
        pid: Pid,
        start_token: Option<StartToken>,
    ) -> bool {
        let Some(child) = UnsupervisedChild::observe_expected(pid, start_token) else {
            return false;
        };
        if !self.panes.contains_key(&pane) && !self.install_terminal(pane, None) {
            return false;
        }
        self.deactivate(pane);
        if let Some(target) = self.panes.get_mut(&pane) {
            target.unsupervised = Some(child);
        }
        self.workspace.set_state(pane, ProcessState::Unsupervised);
        true
    }

    /// Whether `pane` records a child left running unsupervised.
    pub(super) fn is_unsupervised(&self, pane: PaneId) -> bool {
        self.panes
            .get(&pane)
            .is_some_and(|target| target.unsupervised.is_some())
    }

    /// Whether `pane` has a process that may still run: a live handle, one
    /// being launched or adopted, or a child left running unsupervised.
    pub(super) fn holds_process(&self, pane: PaneId) -> bool {
        self.is_live(pane) || self.is_starting(pane) || self.is_unsupervised(pane)
    }

    /// The pid of `pane`'s unsupervised child while that same child runs.
    pub(super) fn running_unsupervised_pid(&self, pane: PaneId) -> Option<Pid> {
        self.panes
            .get(&pane)
            .and_then(|target| target.unsupervised)
            .filter(UnsupervisedChild::is_running)
            .map(|child| child.pid())
    }

    /// Why `pane` is left alone by a project-wide start while its child still
    /// runs unsupervised, naming the pid; `None` once nothing runs there.
    pub(super) fn unsupervised_refusal(&self, pane: PaneId) -> Option<Refusal> {
        let pid = self.running_unsupervised_pid(pane)?;
        let name = self.workspace.process(pane)?.name().clone();
        Some(
            Refusal::builder()
                .name(name)
                .reason(format!("{UNSUPERVISED_START_REFUSED} {pid}"))
                .build(),
        )
    }

    /// Refuses a launch of `pane` while its child may still run unsupervised,
    /// which would put a second copy beside it. A recorded child found gone
    /// is settled first and no longer blocks, unless settling it retired a
    /// row already removed from the project. Returns whether the launch was
    /// refused.
    pub(super) fn refuse_unsupervised_launch(&mut self, pane: PaneId) -> bool {
        if !self.is_unsupervised(pane) {
            return false;
        }
        if let Some(pid) = self.running_unsupervised_pid(pane) {
            self.notify(
                NoticeLevel::Error,
                Some(pane),
                format!("{UNSUPERVISED_START_REFUSED} {pid}"),
            );
            return true;
        }
        let name = self
            .workspace
            .process(pane)
            .map(|process| process.name().clone());
        self.settle_unsupervised_exit(pane);
        if self.workspace.process(pane).is_none() {
            self.notify_process(
                NoticeLevel::Error,
                name,
                UNSUPERVISED_ROW_REMOVED.to_string(),
            );
            return true;
        }
        false
    }

    /// Stops `pane`'s unsupervised child through its process group: the stop
    /// policy's signal, or a kill when `force` is set. The exit is reported
    /// only once the child is gone; until then the generation-guarded
    /// force-stop timer checks every [`UNSUPERVISED_EXIT_POLL`], and kills
    /// once the grace has passed.
    pub(super) fn stop_unsupervised(&mut self, pane: PaneId, force: bool) {
        let policy = (!force).then(|| self.stop_policy_of(pane)).flatten();
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        if !force && !target.exit_intent.accepts_stop_request() {
            return;
        }
        let (signal, grace_deadline) = match policy {
            Some(policy) => (
                unix_signal(*policy.signal()),
                Some(Instant::now() + policy.grace_period().into_inner()),
            ),
            None => (Signal::KILL, None),
        };
        if let Some(child) = target.unsupervised.as_mut() {
            child.begin_stop(grace_deadline);
        }
        let wait = grace_deadline.map_or(UNSUPERVISED_EXIT_POLL, poll_within);
        self.signal_unsupervised(pane, signal, wait);
    }

    /// Follows an unsupervised stop whose check came due: within the grace it
    /// settles a child found gone or checks again, and past it kills the
    /// group and checks again, until the child is gone or
    /// [`UNSUPERVISED_KILL_LIMIT`] kills were sent, when the stop is left to
    /// be retried.
    pub(super) fn escalate_unsupervised(&mut self, pane: PaneId) {
        let Some(child) = self
            .panes
            .get_mut(&pane)
            .and_then(|target| target.unsupervised.as_mut())
        else {
            return;
        };
        if let Some(deadline) = child.grace_deadline() {
            if !child.is_running() {
                self.settle_unsupervised_exit(pane);
                return;
            }
            if Instant::now() < deadline {
                self.check_unsupervised_after(pane, poll_within(deadline));
                return;
            }
            child.end_grace();
        }
        let kills = child.count_kill();
        let pid = child.pid();
        if kills > UNSUPERVISED_KILL_LIMIT && child.is_running() {
            child.begin_stop(None);
            self.give_up_unsupervised_stop(pane);
            self.notify(
                NoticeLevel::Error,
                Some(pane),
                format!("{UNSUPERVISED_KILL_GAVE_UP} {pid}"),
            );
            return;
        }
        self.signal_unsupervised(pane, Signal::KILL, UNSUPERVISED_EXIT_POLL);
    }

    /// Sends `signal` to the unsupervised child's group while it is still the
    /// same child, then settles the exit if it is gone, or waits `wait` before
    /// checking again. A pid now held by another process is never signalled.
    fn signal_unsupervised(&mut self, pane: PaneId, signal: Signal, wait: Duration) {
        let Some(child) = self.panes.get(&pane).and_then(|target| target.unsupervised) else {
            return;
        };
        if !child.is_running() {
            self.settle_unsupervised_exit(pane);
            return;
        }
        let delivered = signal_group(child.pid(), signal).is_ok();
        if !child.is_running() {
            self.settle_unsupervised_exit(pane);
            return;
        }
        if !delivered {
            self.give_up_unsupervised_stop(pane);
            self.notify(
                NoticeLevel::Error,
                Some(pane),
                format!("{UNSUPERVISED_STOP_FAILED} {}", child.pid()),
            );
            return;
        }
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = target.exit_intent.request_stop().stop_delivered();
        }
        self.workspace.set_state(pane, ProcessState::Stopping);
        self.check_unsupervised_after(pane, wait);
    }

    /// Schedules the next check of `pane`'s unsupervised stop after `wait`,
    /// superseding any check already scheduled.
    fn check_unsupervised_after(&mut self, pane: PaneId, wait: Duration) {
        let shutdown_generation = self.advance_shutdown_generation(pane);
        if let (Some(spawn_generation), Some(shutdown_generation)) =
            (self.generations.get(&pane).copied(), shutdown_generation)
        {
            self.schedule_force_stop(pane, spawn_generation, shutdown_generation, wait);
        }
    }

    /// Leaves an unsupervised stop that could not finish for another try.
    fn give_up_unsupervised_stop(&mut self, pane: PaneId) {
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = target.exit_intent.request_stop().stop_delivery_failed();
        }
        self.workspace.set_state(pane, ProcessState::Unsupervised);
    }

    /// Records that `pane`'s unsupervised child is gone: a row removed from
    /// the project retires, any other settles as exited.
    fn settle_unsupervised_exit(&mut self, pane: PaneId) {
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        target.unsupervised = None;
        if target.config_membership == ConfigMembership::RetireOnExit {
            self.retire_pane(pane);
            return;
        }
        target.exit_intent = ExitIntent::FollowPolicy;
        self.bump_generation(pane);
        self.deactivate(pane);
        self.workspace.set_state(pane, ProcessState::Exited);
    }
}

/// How long to wait before the next check of a stop whose grace ends at
/// `deadline`: one poll, or what is left of the grace when that is shorter.
fn poll_within(deadline: Instant) -> Duration {
    UNSUPERVISED_EXIT_POLL.min(deadline.saturating_duration_since(Instant::now()))
}
