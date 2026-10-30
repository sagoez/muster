use std::time::Instant;

use super::entity::Supervisor;
use crate::{
    adapter::service::{BatchOutcome, OccupancyStatus},
    domain::value::{IdleStop, ProcessName},
};

impl Supervisor {
    /// Counts `shells` as the live shells inside, for its status; arming and
    /// cancelling the idle stop follow the leaves and entries the shell
    /// table reports, never this count.
    pub fn set_shells(&mut self, shells: u32) {
        self.occupants = shells;
    }

    /// Arms the idle stop at `now`, as its last shell leaving does.
    pub fn arm_idle(&mut self, now: Instant) {
        self.left_at = Some(now);
    }

    /// Cancels a pending idle stop, since a shell entered or a client
    /// brought the project or one of its processes up; it arms again at the
    /// next leave.
    pub fn hold_up(&mut self) {
        self.left_at = None;
    }

    /// Stops the project through each process's stop policy once its idle
    /// stop is due and it runs anything; a due stop clears either way.
    /// Returns what it stopped.
    pub fn idle_stop_if_due(&mut self, now: Instant) -> Option<BatchOutcome> {
        if self.idle_deadline()? > now {
            return None;
        }
        self.left_at = None;
        let runs_anything = self
            .workspace
            .processes()
            .iter()
            .any(|process| process.state().may_be_running());
        let outcome = runs_anything.then(|| self.down())?;
        self.idle_stopping = outcome.acted().iter().cloned().collect();
        Some(outcome)
    }

    /// Which of `stopping`, the processes an `up` found still stopping, the
    /// last idle stop is stopping; a stop a client asked for is never among
    /// them. Forgets what that idle stop stopped that has since stopped.
    pub fn idle_stopping(&mut self, stopping: &[ProcessName]) -> Vec<ProcessName> {
        self.idle_stopping.retain(|name| stopping.contains(name));
        stopping
            .iter()
            .filter(|name| self.idle_stopping.contains(*name))
            .cloned()
            .collect()
    }

    /// Forgets what the last idle stop stopped, as a client acts on the
    /// project and so owns what runs or stops there from now on.
    pub fn forget_idle_stopping(&mut self) {
        self.idle_stopping.clear();
    }

    /// How long this project keeps running with no shell in it: its own
    /// override, else the setting.
    pub fn idle_stop(&self) -> IdleStop {
        self.project.idle_stop_or(*self.settings.idle_stop())
    }

    /// When the last shell left, while the idle stop is pending.
    pub fn left_at(&self) -> Option<Instant> {
        self.left_at
    }

    /// When the pending idle stop fires under the current effective idle
    /// stop; none when no shell left it or it never stops.
    pub fn idle_deadline(&self) -> Option<Instant> {
        let after = self.idle_stop().after()?;
        self.left_at?.checked_add(after)
    }

    /// Who is in the project as of `now`, and when it stops for want of
    /// anyone.
    pub fn occupancy_status(&self, now: Instant) -> OccupancyStatus {
        OccupancyStatus::builder()
            .shells(self.occupants)
            .idle_stop(self.idle_stop())
            .stops_in(
                self.idle_deadline()
                    .map(|deadline| deadline.saturating_duration_since(now)),
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            config::Stack,
            process::{ProcessState, RestartPolicy},
            project::Project,
            pty::{ExitOutcome, ProcessOutput},
            value::{IdleStop, PaneId, PositiveDuration},
        },
    };

    /// A harness over no processes.
    fn harness() -> Harness {
        Harness::over(Box::new(FakeRunner), &[])
    }

    /// A harness whose process `web` runs, restarting under `restart`.
    fn running(restart: RestartPolicy) -> Harness {
        let mut harness = Harness::over(Box::new(FakeRunner), &[
            spec("web", "true", restart).with_autostart(true)
        ]);
        harness.up();
        harness
    }

    /// The test project with its own idle stop `idle_stop`.
    fn project_with(idle_stop: Option<IdleStop>) -> Project {
        Project::builder()
            .root(test_project().root().clone())
            .name(test_project().name().clone())
            .idle_stop(idle_stop)
            .build()
    }

    /// An idle stop of `seconds`.
    fn after(seconds: u64) -> IdleStop {
        IdleStop::After(PositiveDuration::try_new(Duration::from_secs(seconds)).unwrap())
    }

    /// The setting's idle stop the test settings carry.
    fn setting() -> Duration {
        test_settings().idle_stop().after().unwrap()
    }

    /// The shell count is only shown: no count arms or cancels anything.
    #[test]
    fn the_shell_count_is_shown_and_arms_nothing() {
        let mut harness = harness();
        let supervisor = harness.supervisor_mut();
        let now = Instant::now();

        supervisor.set_shells(2);
        supervisor.set_shells(0);

        assert_eq!(supervisor.idle_deadline(), None, "a count is no leave");
        assert_eq!(supervisor.occupancy_status(now).shells(), 0);
        supervisor.set_shells(1);
        assert_eq!(supervisor.occupancy_status(now).shells(), 1);
    }

    /// A leave arms the stop from its instant, and an entry or an up
    /// cancels it.
    #[test]
    fn arm_idle_arms_from_the_leave_and_hold_up_cancels() {
        let mut harness = harness();
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();

        supervisor.arm_idle(left);
        assert_eq!(supervisor.idle_deadline(), Some(left + setting()));
        supervisor.set_shells(0);
        assert_eq!(
            supervisor.idle_deadline(),
            Some(left + setting()),
            "a recount keeps the leave instant"
        );

        supervisor.hold_up();
        assert_eq!(supervisor.idle_deadline(), None);
    }

    #[test]
    fn a_due_idle_stop_stops_what_runs_once() {
        let mut harness = running(RestartPolicy::Never);
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.arm_idle(left);
        let due = left + setting();

        assert_eq!(
            supervisor.idle_stop_if_due(due - Duration::from_secs(1)),
            None
        );
        let outcome = supervisor.idle_stop_if_due(due).unwrap();

        assert_eq!(outcome.acted(), &[name("web")]);
        assert_eq!(supervisor.idle_deadline(), None, "the stop clears it");
        assert_eq!(supervisor.idle_stop_if_due(due), None);
        assert_ne!(
            *supervisor.workspace().processes()[0].state(),
            ProcessState::Running,
            "stopped through its stop policy"
        );
    }

    /// Only what the idle stop stopped counts as idle stopping, until it has
    /// stopped or a client acts on the project.
    #[test]
    fn idle_stopping_names_only_what_the_idle_stop_stopped() {
        let mut harness = running(RestartPolicy::Never);
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.arm_idle(left);
        let other = name("other");

        assert!(supervisor.idle_stopping(&[name("web")]).is_empty());
        supervisor.idle_stop_if_due(left + setting()).unwrap();

        assert_eq!(supervisor.idle_stopping(&[name("web"), other.clone()]), [
            name("web")
        ]);
        supervisor.forget_idle_stopping();
        assert!(supervisor.idle_stopping(&[name("web")]).is_empty());

        supervisor.idle_stopping = [name("web")].into();
        assert!(supervisor.idle_stopping(&[other]).is_empty(), "web stopped");
        assert!(supervisor.idle_stopping(&[name("web")]).is_empty());
    }

    #[test]
    fn a_due_idle_stop_of_a_project_running_nothing_only_clears() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[spec(
            "web",
            "true",
            RestartPolicy::Never,
        )]);
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.arm_idle(left);

        assert_eq!(supervisor.idle_stop_if_due(left + setting()), None);
        assert_eq!(supervisor.idle_deadline(), None);
    }

    #[test]
    fn idle_stop_never_arms_nothing() {
        let mut harness = running(RestartPolicy::Never);
        let supervisor = harness.supervisor_mut();
        supervisor.project = project_with(Some(IdleStop::Never));
        let left = Instant::now();

        supervisor.arm_idle(left);

        assert_eq!(supervisor.idle_deadline(), None);
        assert_eq!(supervisor.occupancy_status(left).stops_in(), None);
        assert_eq!(supervisor.idle_stop_if_due(left + setting() * 100), None);
        assert_eq!(
            *supervisor.workspace().processes()[0].state(),
            ProcessState::Running
        );
    }

    /// Review Focus 6: the deadline follows the setting as it is now, from
    /// the same leave instant.
    #[test]
    fn a_settings_change_moves_a_pending_deadline() {
        let mut harness = running(RestartPolicy::Never);
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.arm_idle(left);

        supervisor.apply_settings(&test_settings().with_idle_stop(after(1)));

        assert_eq!(
            supervisor.idle_deadline(),
            Some(left + Duration::from_secs(1))
        );
        assert!(
            supervisor
                .idle_stop_if_due(left + Duration::from_secs(1))
                .is_some()
        );
    }

    /// A project's own idle stop, applied with its stack, beats the setting
    /// and moves a pending deadline at once; clearing it follows the
    /// setting again.
    #[test]
    fn an_override_applied_with_the_stack_moves_a_pending_deadline() {
        let mut harness = running(RestartPolicy::Never);
        let processes = vec![spec("web", "true", RestartPolicy::Never).with_autostart(true)];
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.arm_idle(left);

        supervisor.apply_stack(
            &Stack::builder()
                .project(project_with(Some(after(2))))
                .processes(processes.clone())
                .build(),
        );
        assert_eq!(supervisor.idle_stop(), after(2));
        assert_eq!(
            supervisor.idle_deadline(),
            Some(left + Duration::from_secs(2))
        );

        supervisor.apply_stack(
            &Stack::builder()
                .project(project_with(None))
                .processes(processes)
                .build(),
        );
        assert_eq!(supervisor.idle_deadline(), Some(left + setting()));
    }

    /// A restart its policy makes is no client bringing the project up: the
    /// pending stop stands.
    #[test]
    fn a_policy_respawn_keeps_a_pending_timer() {
        let mut harness = running(RestartPolicy::Always);
        let left = Instant::now();
        harness.supervisor_mut().arm_idle(left);
        let pane = PaneId::new(0);
        let generation = harness.supervisor().generations()[&pane];

        harness.supervisor_mut().handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        let respawn = harness.supervisor().generations()[&pane];
        harness.supervisor_mut().handle_respawn(pane, respawn);

        assert_eq!(
            *harness.supervisor().workspace().processes()[0].state(),
            ProcessState::Running,
            "it was respawned"
        );
        assert_eq!(harness.supervisor().idle_deadline(), Some(left + setting()));
    }

    #[test]
    fn a_fresh_supervisor_reports_no_shells_and_the_setting_s_idle_stop() {
        let harness = harness();
        let status = harness.supervisor().occupancy_status(Instant::now());

        assert_eq!(status.shells(), 0);
        assert_eq!(status.idle_stop(), *test_settings().idle_stop());
        assert_eq!(status.stops_in(), None);
        assert_eq!(harness.supervisor().idle_deadline(), None);
        assert_eq!(
            harness.supervisor().snapshot().occupancy(),
            &status,
            "ps carries the occupancy"
        );
    }

    #[test]
    fn a_project_override_is_the_occupancy_idle_stop() {
        let mut harness = harness();
        let supervisor = harness.supervisor_mut();
        supervisor.project = Project::builder()
            .root(test_project().root().clone())
            .name(test_project().name().clone())
            .idle_stop(Some(IdleStop::Never))
            .build();

        assert_eq!(supervisor.idle_stop(), IdleStop::Never);
        assert_eq!(
            supervisor.occupancy_status(Instant::now()).idle_stop(),
            IdleStop::Never
        );
    }

    /// A project its last shell left counts down from that instant.
    #[test]
    fn a_left_project_counts_down_from_its_leaving() {
        let mut harness = harness();
        let supervisor = harness.supervisor_mut();
        let left = Instant::now();
        supervisor.left_at = Some(left);
        let after = test_settings().idle_stop().after().unwrap();
        let later = left + Duration::from_secs(60);

        assert_eq!(supervisor.idle_deadline(), Some(left + after));
        assert_eq!(
            supervisor.occupancy_status(later).stops_in(),
            Some(after - Duration::from_secs(60))
        );
        assert_eq!(
            supervisor.occupancy_status(left + after * 2).stops_in(),
            Some(Duration::ZERO),
            "an overdue stop reads as now"
        );

        supervisor.project = Project::builder()
            .root(test_project().root().clone())
            .name(test_project().name().clone())
            .idle_stop(Some(IdleStop::Never))
            .build();
        assert_eq!(supervisor.idle_deadline(), None);
        assert_eq!(supervisor.occupancy_status(later).stops_in(), None);
    }
}
