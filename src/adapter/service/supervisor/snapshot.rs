use std::{collections::BTreeSet, time::Instant};

use super::{config_membership::ConfigMembership, entity::Supervisor};
use crate::{
    adapter::service::{ProcessStatus, ProjectStatus, StackUrl, UrlSource},
    domain::{
        port::ListeningSocket,
        process::{ProcessState, ReadyCheck, ReadyProbe},
    },
};

/// What a detected port's URL starts with, before the port.
const DETECTED_URL_PREFIX: &str = "http://localhost:";

impl Supervisor {
    /// A point-in-time view of every process and of the shells in the
    /// project, for `ps`. An unsupervised child
    /// found gone is shown as it will settle: exited, or no row at all when its
    /// row was removed from the stack, so `ps` never shows an unsupervised
    /// process with no pid.
    pub fn snapshot(&self) -> ProjectStatus {
        let processes = self
            .workspace
            .processes()
            .iter()
            .filter_map(|process| {
                let pane = *process.id();
                let mut state = *process.state();
                if state == ProcessState::Unsupervised
                    && self.running_unsupervised_pid(pane).is_none()
                {
                    let retiring = self.panes.get(&pane).is_some_and(|target| {
                        target.config_membership() == ConfigMembership::RetireOnExit
                    });
                    if retiring {
                        return None;
                    }
                    state = ProcessState::Exited;
                }
                let live = self
                    .panes
                    .get(&pane)
                    .filter(|target| target.handle().is_some());
                let ports = self.metrics.ports(pane);
                Some(
                    ProcessStatus::builder()
                        .name(process.name().clone())
                        .state(state)
                        .ready(
                            self.readiness
                                .get(&pane)
                                .map(|readiness| readiness.state())
                                .unwrap_or_default(),
                        )
                        .urls(urls(process.ready().as_ref(), &ports))
                        .ports(ports)
                        .run_start(self.output_position(pane).1)
                        .pid(
                            live.and_then(|target| {
                                target
                                    .handle()
                                    .as_ref()
                                    .and_then(|handle| handle.process_id())
                            })
                            .or_else(|| self.running_unsupervised_pid(pane)),
                        )
                        .uptime(live.map(|target| target.started_at().elapsed()))
                        .restarts(self.restarts.restarts(pane))
                        .usage(self.metrics.usage(pane))
                        .attached(self.attach.attached(pane))
                        .last_exit(
                            (!state.is_active() || state == ProcessState::Restarting)
                                .then(|| self.restarts.last_exit(pane))
                                .flatten(),
                        )
                        .stop_grace(process.stop().grace_period().into_inner())
                        .blocked_on(
                            self.metrics
                                .block(pane)
                                .filter(|_| !state.may_be_running())
                                .cloned(),
                        )
                        .build(),
                )
            })
            .collect();
        ProjectStatus::builder()
            .project(self.project.clone())
            .processes(processes)
            .occupancy(self.occupancy_status(Instant::now()))
            .build()
    }
}

/// The URLs a process serves: the one its readiness check declares, else
/// `http://localhost:<port>` for each port it listens on.
fn urls(check: Option<&ReadyCheck>, ports: &[ListeningSocket]) -> Vec<StackUrl> {
    if let Some(ReadyProbe::Http(url)) = check.map(ReadyCheck::probe) {
        return vec![
            StackUrl::builder()
                .url(url.to_string())
                .source(UrlSource::Declared)
                .build(),
        ];
    }
    let distinct: BTreeSet<u16> = ports.iter().map(ListeningSocket::port).collect();
    distinct
        .into_iter()
        .map(|port| {
            StackUrl::builder()
                .url(format!("{DETECTED_URL_PREFIX}{port}"))
                .source(UrlSource::Detected)
                .build()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            process::{ProcessState, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::PaneId,
        },
    };

    const PANE: u64 = 0;

    /// `new` builds the workspace from the stack and starts nothing.
    #[test]
    fn a_new_supervisor_runs_the_stack_and_starts_nothing() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[spec(
            "p",
            "true",
            RestartPolicy::Never,
        )]);

        let status = harness.supervisor_mut().snapshot();

        assert_eq!(status.project(), harness.supervisor_mut().project());
        assert_eq!(status.processes().len(), 1);
        assert_eq!(status.processes()[0].name(), &name("p"));
        assert_eq!(*status.processes()[0].state(), ProcessState::Pending);
        assert_eq!(*status.processes()[0].stop_grace(), TEST_STOP_GRACE);
    }

    /// `ps` reports a restarted process's restarts, its uptime and its attached
    /// clients.
    #[test]
    fn the_snapshot_reports_restarts_uptime_and_attached_clients() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[spec(
            "p",
            "true",
            RestartPolicy::Always,
        )]);
        harness.up();
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor_mut().generations()[&pane];
        harness.supervisor_mut().handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        let respawn = harness.supervisor_mut().generations()[&pane];
        harness.supervisor_mut().handle_respawn(pane, respawn);
        harness.supervisor_mut().set_attached(&name("p"), 2);

        let status = harness.supervisor_mut().snapshot();

        let process = &status.processes()[0];
        assert_eq!(*process.state(), ProcessState::Running);
        assert_eq!(*process.restarts(), 1);
        assert!(process.uptime().is_some());
        assert_eq!(*process.attached(), 2);
    }
}
