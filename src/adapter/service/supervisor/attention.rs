use std::time::{Duration, Instant};

use super::entity::Supervisor;
use crate::{
    adapter::{service::NoticeLevel, terminal::Signal},
    domain::{
        notification::{Notification, NotificationId},
        process::{ActivityState, ProcessKind},
        value::PaneId,
    },
};

/// Notice for a bare bell that carried no text of its own.
const AWAITING_INPUT_NOTICE: &str = "awaiting input";
/// Minimum gap between bell notifications from one pane, absorbing a burst (e.g.
/// shell tab-completion) into a single alert.
const BELL_THROTTLE: Duration = Duration::from_secs(3);
/// Quiet interval before a detached terminal's bell becomes a desktop alert.
pub(super) const BELL_ESCALATION_DELAY: Duration = Duration::from_secs(2);

impl Supervisor {
    /// Turns desktop notifications on or off, for a test. Notices are raised
    /// either way.
    #[cfg(test)]
    pub(crate) fn set_desktop_notifications(&mut self, enabled: bool) {
        self.desktop_notifications = enabled;
    }

    /// Whether a writable client is attached to `pane`'s terminal, so the user
    /// is already looking at it.
    fn is_attached(&self, pane: PaneId) -> bool {
        self.attached.contains_key(&pane)
    }

    /// Applies one decoded terminal signal from `pane`: updates its inferred
    /// activity and raises or closes the attention it asks for.
    pub(super) fn apply_signal(&mut self, pane: PaneId, signal: Signal) {
        match signal {
            Signal::Output => {
                let now = Instant::now();
                let activity = self.panes.get_mut(&pane).and_then(|target| {
                    if let Some(deadline) = target.pending_bell_notification.as_mut() {
                        *deadline = now + BELL_ESCALATION_DELAY;
                        None
                    } else {
                        target.activity.observe_output(now)
                    }
                });
                self.record_activity(pane, activity);
            },
            Signal::Title(title) => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .and_then(|target| target.activity.observe_title(Instant::now(), title));
                self.record_activity(pane, activity);
            },
            Signal::Progress(active) => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .map(|target| target.activity.observe_progress(active));
                self.record_activity(pane, activity);
            },
            Signal::Notify {
                identifier,
                title,
                body,
            } => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .map(|target| target.activity.observe_attention());
                self.record_activity(pane, activity);
                let is_bell = title.is_none() && body.is_none();
                if is_bell
                    && (!self.accept_bell(pane) || self.defer_or_suppress_non_agent_bell(pane))
                {
                    return;
                }
                self.raise_notification(pane, identifier, title, body);
            },
            Signal::Close { identifier } => self.close_notification(pane, &identifier),
        }
    }

    /// Records `pane`'s new inferred activity, when a signal changed it.
    fn record_activity(&mut self, pane: PaneId, activity: Option<ActivityState>) {
        if let Some(activity) = activity {
            self.workspace.set_activity(pane, activity);
        }
    }

    /// The nearest time at which quiet output turns idle or a deferred bell
    /// escalates.
    pub fn next_activity_deadline(&self) -> Option<Instant> {
        self.panes
            .values()
            .flat_map(|pane| [pane.activity.deadline(), pane.pending_bell_notification])
            .flatten()
            .min()
    }

    /// Expires quiet-output activity and escalates every deferred bell that is
    /// due on a terminal still detached. Returns whether any pane changed.
    pub fn expire_quiet_activity(&mut self, now: Instant) -> bool {
        let expired = self
            .panes
            .iter_mut()
            .filter_map(|(pane, target)| {
                target
                    .activity
                    .expire(now)
                    .map(|activity| (*pane, activity))
            })
            .collect::<Vec<_>>();
        for (pane, activity) in &expired {
            self.workspace.set_activity(*pane, *activity);
        }
        let due_bells = self
            .panes
            .iter_mut()
            .filter_map(|(pane, target)| {
                let due = target
                    .pending_bell_notification
                    .is_some_and(|deadline| deadline <= now);
                if due {
                    target.pending_bell_notification = None;
                }
                due.then_some(*pane)
            })
            .collect::<Vec<_>>();
        for pane in &due_bells {
            if !self.is_attached(*pane) {
                self.raise_notification(*pane, None, None, None);
            }
        }
        !expired.is_empty() || !due_bells.is_empty()
    }

    /// Raises an attention notice for `pane`, and a desktop notification scoped
    /// to its terminal lifetime when desktop notifications are on.
    fn raise_notification(
        &mut self,
        pane: PaneId,
        identifier: Option<NotificationId>,
        title: Option<String>,
        body: Option<String>,
    ) {
        let is_bell = title.is_none() && body.is_none();
        let Some(scope) = self
            .panes
            .get(&pane)
            .map(|target| target.notification_scope)
        else {
            return;
        };
        let Some(process) = self.workspace.process(pane) else {
            return;
        };
        let name = process.name().clone();
        let desktop_body = is_bell
            .then(|| AWAITING_INPUT_NOTICE.to_string())
            .or(body.clone());
        let message = body
            .or_else(|| title.clone())
            .unwrap_or_else(|| AWAITING_INPUT_NOTICE.to_string());
        self.notify_process(NoticeLevel::Attention, Some(name.clone()), message);
        let notification = Notification::builder()
            .pane(pane)
            .scope(scope)
            .source(name)
            .title(title)
            .body(desktop_body)
            .identifier(identifier)
            .build();
        if self.desktop_notifications
            && let Some(notifier) = &self.notifier
        {
            notifier.notify(&notification);
        }
    }

    /// Accepts a bare bell unless another bell from this pane arrived too
    /// recently.
    fn accept_bell(&mut self, pane: PaneId) -> bool {
        let Some(target) = self.panes.get_mut(&pane) else {
            return false;
        };
        let now = Instant::now();
        if target
            .last_bell
            .is_some_and(|last| now.duration_since(last) < BELL_THROTTLE)
        {
            return false;
        }
        target.last_bell = Some(now);
        true
    }

    /// Holds back a non-agent bell: one on an attached terminal stays an
    /// activity marker, and one on a detached terminal escalates after a quiet
    /// delay.
    /// Returns whether the bell was held back.
    fn defer_or_suppress_non_agent_bell(&mut self, pane: PaneId) -> bool {
        let Some(process) = self.workspace.process(pane) else {
            return false;
        };
        if *process.kind() == ProcessKind::Agent {
            return false;
        }
        if !self.is_attached(pane)
            && let Some(target) = self.panes.get_mut(&pane)
        {
            target.pending_bell_notification = Some(Instant::now() + BELL_ESCALATION_DELAY);
        }
        true
    }

    /// Closes one prior identified desktop notification for `pane`'s terminal
    /// lifetime.
    fn close_notification(&self, pane: PaneId, identifier: &NotificationId) {
        if let Some(scope) = self
            .panes
            .get(&pane)
            .map(|target| target.notification_scope)
            && let Some(notifier) = &self.notifier
        {
            notifier.close(pane, scope, identifier);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::{service::supervisor::testing::*, terminal::activity::OUTPUT_IDLE_TIMEOUT},
        domain::{
            process::{Process, ProcessState, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::{CommandLine, ProcessName},
        },
    };

    const PANE: u64 = 0;
    const BACKGROUND: u64 = 1;
    const BELL: &[u8] = b"\x07";

    /// The name of the process on `pane`.
    fn name_of(pane: u64) -> ProcessName {
        ProcessName::try_new(format!("p{pane}")).unwrap()
    }

    /// One process of `kind` on `pane` with `restart` policy.
    fn process(pane: u64, kind: ProcessKind, restart: RestartPolicy) -> Process {
        Process::builder()
            .id(PaneId::new(pane))
            .name(name_of(pane))
            .kind(kind)
            .command(Some(CommandLine::try_new("sh").unwrap()))
            .restart(restart)
            .build()
    }

    /// A started supervisor over `processes` that delivers desktop
    /// notifications to the returned recorder.
    fn attending(processes: Vec<Process>) -> (Harness, RecordingNotifier) {
        let mut harness = Harness::new(Box::new(FakeRunner), processes);
        let notifier = RecordingNotifier::default();
        harness.supervisor.set_notifier(Box::new(notifier.clone()));
        harness.supervisor.set_desktop_notifications(true);
        harness.supervisor.up();
        (harness, notifier)
    }

    /// A started terminal on [`PANE`], delivering desktop notifications.
    fn terminal() -> (Harness, RecordingNotifier) {
        attending(vec![process(
            PANE,
            ProcessKind::Terminal,
            RestartPolicy::Never,
        )])
    }

    /// Feeds `output` to `pane`'s current process.
    fn feed(harness: &mut Harness, pane: u64, output: ProcessOutput) {
        let pane = PaneId::new(pane);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(pane, generation, output);
    }

    /// Feeds `bytes` as one chunk of `pane`'s output.
    fn chunk(harness: &mut Harness, pane: u64, bytes: &[u8]) {
        feed(harness, pane, ProcessOutput::Chunk(bytes.to_vec()));
    }

    /// The inferred activity of `pane`'s process.
    fn activity(harness: &Harness, pane: u64) -> ActivityState {
        *harness
            .supervisor
            .workspace()
            .process(PaneId::new(pane))
            .unwrap()
            .activity()
    }

    /// The messages of the attention notices raised so far, oldest first.
    fn attention(harness: &Harness) -> Vec<(Option<ProcessName>, String)> {
        harness
            .notices
            .lock()
            .unwrap()
            .recent(harness.supervisor.project())
            .into_iter()
            .filter(|notice| *notice.level() == NoticeLevel::Attention)
            .map(|notice| (notice.process().clone(), notice.message().clone()))
            .collect()
    }

    #[test]
    fn output_marks_the_process_working() {
        let (mut harness, _) = terminal();
        chunk(&mut harness, PANE, b"compiling...");
        assert_eq!(activity(&harness, PANE), ActivityState::Working);
    }

    #[test]
    fn ordinary_output_returns_to_idle_after_becoming_quiet() {
        let (mut harness, _) = terminal();
        chunk(&mut harness, PANE, b"prompt> ");
        let deadline = harness
            .supervisor
            .next_activity_deadline()
            .expect("ordinary output schedules an idle transition");

        assert!(harness.supervisor.expire_quiet_activity(deadline));
        assert_eq!(activity(&harness, PANE), ActivityState::Idle);
        assert!(harness.supervisor.next_activity_deadline().is_none());
    }

    #[test]
    fn explicit_progress_stays_working_until_a_protocol_update() {
        let (mut harness, _) = terminal();
        chunk(&mut harness, PANE, b"\x1b]9;4;1;40\x07");
        chunk(&mut harness, PANE, b"still working");

        assert!(harness.supervisor.next_activity_deadline().is_none());
        assert!(
            !harness
                .supervisor
                .expire_quiet_activity(Instant::now() + OUTPUT_IDLE_TIMEOUT)
        );
        assert_eq!(activity(&harness, PANE), ActivityState::Working);
    }

    /// The brief's rule: a bell on a terminal someone is attached to stays a
    /// marker, while one on a detached terminal escalates to a single
    /// desktop alert once it has been quiet.
    #[test]
    fn bell_on_an_attached_process_is_suppressed_and_on_a_detached_one_notifies() {
        let (mut harness, notifier) = attending(vec![
            process(PANE, ProcessKind::Terminal, RestartPolicy::Never),
            process(BACKGROUND, ProcessKind::Terminal, RestartPolicy::Never),
        ]);
        harness.supervisor.set_attached(&name_of(PANE), 1);

        chunk(&mut harness, PANE, BELL);
        chunk(&mut harness, BACKGROUND, BELL);

        assert_eq!(activity(&harness, PANE), ActivityState::AwaitingInput);
        assert_eq!(activity(&harness, BACKGROUND), ActivityState::AwaitingInput);
        assert_eq!(notifier.count(), 0, "a bell is first only a marker");
        let deadline = harness
            .supervisor
            .next_activity_deadline()
            .expect("the detached bell has an escalation deadline");
        assert!(harness.supervisor.expire_quiet_activity(deadline));
        assert!(
            !harness
                .supervisor
                .expire_quiet_activity(deadline + BELL_ESCALATION_DELAY)
        );

        assert_eq!(notifier.panes(), [PaneId::new(BACKGROUND)]);
        assert_eq!(notifier.bodies(), [Some(AWAITING_INPUT_NOTICE.to_string())]);
        assert_eq!(attention(&harness), [(
            Some(name_of(BACKGROUND)),
            AWAITING_INPUT_NOTICE.to_string()
        )]);
    }

    /// A deferred bell does not escalate once a client attaches to its
    /// terminal, since the user is now looking at it.
    #[test]
    fn attaching_before_the_escalation_deadline_suppresses_the_alert() {
        let (mut harness, notifier) = terminal();
        chunk(&mut harness, PANE, BELL);
        let deadline = harness.supervisor.next_activity_deadline().unwrap();

        harness.supervisor.set_attached(&name_of(PANE), 1);

        assert!(harness.supervisor.expire_quiet_activity(deadline));
        assert_eq!(notifier.count(), 0);
        assert!(attention(&harness).is_empty());
    }

    /// Detaching the last writable client makes a later bell escalate again.
    #[test]
    fn a_bell_after_the_last_client_detaches_escalates() {
        let (mut harness, notifier) = terminal();
        harness.supervisor.set_attached(&name_of(PANE), 1);
        harness.supervisor.set_attached(&name_of(PANE), 0);

        chunk(&mut harness, PANE, BELL);
        let deadline = harness.supervisor.next_activity_deadline().unwrap();
        harness.supervisor.expire_quiet_activity(deadline);

        assert_eq!(notifier.count(), 1);
    }

    #[test]
    fn an_attached_terminal_bell_marks_awaiting_input_without_notifying() {
        let (mut harness, notifier) = terminal();
        harness.supervisor.set_attached(&name_of(PANE), 1);

        chunk(&mut harness, PANE, BELL);

        assert_eq!(activity(&harness, PANE), ActivityState::AwaitingInput);
        assert!(
            attention(&harness).is_empty(),
            "the marker is sufficient while attached"
        );
        assert_eq!(notifier.count(), 0, "attached shell bells stay local");
        assert!(harness.supervisor.next_activity_deadline().is_none());
    }

    #[test]
    fn a_title_only_notification_has_no_duplicate_desktop_body() {
        let (mut harness, notifier) = terminal();

        chunk(&mut harness, PANE, b"\x1b]99;;Hello world\x1b\\");

        assert_eq!(attention(&harness), [(
            Some(name_of(PANE)),
            "Hello world".to_string()
        )]);
        assert_eq!(notifier.bodies(), [None]);
    }

    /// kitty/other OSC notifications close with ST (`ESC \`); the terminator
    /// must not flip the pane back to working and lose its attention marker.
    #[test]
    fn an_st_terminated_notification_keeps_the_awaiting_marker() {
        let (mut harness, _) = terminal();
        chunk(&mut harness, PANE, b"\x1b]777;notify;agent;done\x1b\\");
        assert_eq!(activity(&harness, PANE), ActivityState::AwaitingInput);
    }

    #[test]
    fn a_burst_of_attached_bells_does_not_notify() {
        let (mut harness, notifier) = terminal();
        harness.supervisor.set_attached(&name_of(PANE), 1);

        for _ in 0..5 {
            chunk(&mut harness, PANE, BELL);
        }

        assert_eq!(notifier.count(), 0, "attached shell bells stay local");
    }

    /// A burst of bells on a detached terminal escalates once, not per bell.
    #[test]
    fn a_burst_of_detached_bells_escalates_once() {
        let (mut harness, notifier) = terminal();

        for _ in 0..5 {
            chunk(&mut harness, PANE, BELL);
        }
        let deadline = harness.supervisor.next_activity_deadline().unwrap();
        harness.supervisor.expire_quiet_activity(deadline);

        assert_eq!(notifier.count(), 1);
    }

    /// A terminal that exits during the bell grace period must not raise a
    /// late desktop alert for a process that can no longer need input.
    #[test]
    fn an_exited_detached_terminal_cancels_its_pending_bell() {
        let (mut harness, notifier) = terminal();

        chunk(&mut harness, PANE, BELL);
        feed(
            &mut harness,
            PANE,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert!(
            !harness
                .supervisor
                .expire_quiet_activity(Instant::now() + BELL_ESCALATION_DELAY)
        );
        assert_eq!(notifier.count(), 0);
    }

    /// Agent bells stay immediate, attached or not, because they are part of
    /// the agent attention contract.
    #[test]
    fn an_agent_bell_notifies_immediately() {
        let (mut harness, notifier) = attending(vec![process(
            PANE,
            ProcessKind::Agent,
            RestartPolicy::Never,
        )]);
        harness.supervisor.set_attached(&name_of(PANE), 1);

        chunk(&mut harness, PANE, BELL);

        assert_eq!(notifier.bodies(), [Some(AWAITING_INPUT_NOTICE.to_string())]);
    }

    #[test]
    fn kitty_update_and_close_identifiers_reach_the_notifier() {
        let (mut harness, notifier) = terminal();

        chunk(
            &mut harness,
            PANE,
            b"\x1b]99;i=build;first\x1b\\\x1b]99;i=build;second\x1b\\\x1b]99;i=build:p=close;\x1b\\",
        );

        assert_eq!(
            notifier
                .identifiers()
                .into_iter()
                .flatten()
                .map(|identifier| identifier.to_string())
                .collect::<Vec<_>>(),
            ["build", "build"]
        );
        assert_eq!(
            notifier
                .closed()
                .into_iter()
                .map(|(pane, identifier)| (pane, identifier.to_string()))
                .collect::<Vec<_>>(),
            [(PaneId::new(PANE), "build".to_string())]
        );
    }

    #[test]
    fn desktop_off_suppresses_the_notification_but_not_the_notice() {
        let (mut harness, notifier) = terminal();
        harness.supervisor.set_desktop_notifications(false);

        chunk(&mut harness, PANE, b"\x1b]9;attention requested\x07");

        assert_eq!(attention(&harness), [(
            Some(name_of(PANE)),
            "attention requested".to_string()
        )]);
        assert_eq!(
            notifier.count(),
            0,
            "no desktop notification while the setting is off"
        );
    }

    #[test]
    fn a_restart_clears_the_awaiting_input_marker() {
        let (mut harness, _) = attending(vec![process(
            PANE,
            ProcessKind::Terminal,
            RestartPolicy::Always,
        )]);
        chunk(&mut harness, PANE, BELL);
        assert_eq!(activity(&harness, PANE), ActivityState::AwaitingInput);

        feed(
            &mut harness,
            PANE,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert_eq!(
            *harness
                .supervisor
                .workspace()
                .process(PaneId::new(PANE))
                .unwrap()
                .state(),
            ProcessState::Restarting
        );
        assert_eq!(
            activity(&harness, PANE),
            ActivityState::Idle,
            "a process in restart backoff shows no attention marker"
        );
    }
}
