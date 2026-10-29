use std::time::Instant;

use super::entity::Supervisor;
use crate::{adapter::service::OccupancyStatus, domain::value::IdleStop};

impl Supervisor {
    /// How long this project keeps running with no shell in it: its own
    /// override, else the setting.
    pub fn idle_stop(&self) -> IdleStop {
        self.project.idle_stop_or(*self.settings.idle_stop())
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
        domain::{project::Project, value::IdleStop},
    };

    /// A harness over no processes.
    fn harness() -> Harness {
        Harness::over(Box::new(FakeRunner), &[])
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
