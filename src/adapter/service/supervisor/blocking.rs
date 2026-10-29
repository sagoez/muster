//! Refusing a start whose port another supervised process already listens
//! on, so a second dev server does not fail on `EADDRINUSE` while the first
//! one keeps serving.

use std::collections::HashMap;

use super::entity::Supervisor;
use crate::{
    adapter::service::{EventKind, NoticeLevel, PortBlock, PortHolder},
    domain::value::PaneId,
};

/// Opens the notice of a start refused for a held port, before the port.
const BLOCKED_NOTICE: &str = "did not start: port";
/// Between the port and who holds it in that notice.
const HELD_BY: &str = "is held by";

impl Supervisor {
    /// The ports this project's live processes listen on, each with who
    /// holds it.
    pub fn held_ports(&self) -> Vec<(u16, PortHolder)> {
        self.ports
            .iter()
            .filter_map(|(pane, sockets)| {
                let process = self.workspace.process(*pane)?;
                let holder = PortHolder::builder()
                    .project(self.project.name().clone())
                    .process(process.name().clone())
                    .build();
                Some(
                    sockets
                        .iter()
                        .map(move |socket| (socket.port(), holder.clone())),
                )
            })
            .flatten()
            .collect()
    }

    /// Takes `held`, every port a supervised process of any project listens
    /// on, which a start checks its own port against.
    pub fn set_held_ports(&mut self, held: HashMap<u16, PortHolder>) {
        self.held_ports = held;
    }

    /// Refuses to launch `pane` when another process listens on the port its
    /// readiness check names: the refusal is an error notice, a `blocked`
    /// event and the pane's `blocked_on`. Returns whether it refused.
    pub(super) fn refuse_blocked_launch(&mut self, pane: PaneId) -> bool {
        let Some(process) = self.workspace.process(pane) else {
            return false;
        };
        let me = PortHolder::builder()
            .project(self.project.name().clone())
            .process(process.name().clone())
            .build();
        let block = process
            .ready()
            .as_ref()
            .and_then(|check| check.probe().port())
            .and_then(|port| {
                self.held_ports
                    .get(&port.into_inner())
                    .filter(|holder| **holder != me)
                    .map(|holder| {
                        PortBlock::builder()
                            .port(port)
                            .holder(holder.clone())
                            .build()
                    })
            });
        let Some(block) = block else {
            self.blocks.remove(&pane);
            return false;
        };
        let message = format!(
            "{BLOCKED_NOTICE} {} {HELD_BY} {}",
            block.port(),
            block.holder()
        );
        self.notify(NoticeLevel::Error, Some(pane), message.clone());
        self.record_event(Some(pane), EventKind::Blocked, message);
        self.blocks.insert(pane, block);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use super::*;
    use crate::{
        adapter::service::{LaunchEnvironment, supervisor::testing::*},
        domain::{
            config::ProcessSpec,
            port::ListeningSocket,
            process::{ProcessState, ReadyCheck, ReadyProbe, RestartPolicy},
            value::{LocalPort, PositiveDuration, ProjectName},
        },
    };

    /// The port every process here declares.
    const PORT: u16 = 3000;

    /// `raw`, checking port [`PORT`] for readiness.
    fn on_port(raw: &str) -> ProcessSpec {
        spec(raw, "true", RestartPolicy::Never).with_ready(Some(
            ReadyCheck::builder()
                .probe(ReadyProbe::Port(LocalPort::try_new(PORT).unwrap()))
                .timeout(PositiveDuration::try_new(Duration::from_secs(30)).unwrap())
                .build(),
        ))
    }

    /// `process` of the project `project`, as a port's holder.
    fn holder(project: &str, process: &str) -> PortHolder {
        PortHolder::builder()
            .project(ProjectName::try_new(project).unwrap())
            .process(name(process))
            .build()
    }

    /// A port another project's process holds refuses the start: nothing is
    /// spawned, the refusal names the holder in the batch, the notices and a
    /// `blocked` event, and the status says what blocks it until it starts.
    #[test]
    fn a_start_on_a_held_port_is_refused_naming_the_holder() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("web")]);
        harness
            .supervisor
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));

        let batch = harness.supervisor.up(&LaunchEnvironment::default());

        assert!(batch.acted().is_empty());
        assert_eq!(
            batch.refused()[0].reason(),
            "did not start: port 3000 is held by project 'admin', process 'web'"
        );
        let status = harness.supervisor.snapshot();
        let web = &status.processes()[0];
        assert_eq!(*web.state(), ProcessState::Pending);
        let block = web.blocked_on().as_ref().expect("blocked");
        assert_eq!(block.holder(), &holder("admin", "web"));
        assert_eq!(
            harness
                .event_log
                .lock()
                .unwrap()
                .page(harness.supervisor.root(), None, &[EventKind::Blocked])
                .events()
                .len(),
            1
        );

        harness.supervisor.set_held_ports(HashMap::new());
        harness.supervisor.up(&LaunchEnvironment::default());
        let status = harness.supervisor.snapshot();
        assert!(status.processes()[0].blocked_on().is_none());
        assert!(status.processes()[0].state().may_be_running());
    }

    /// A process's own port never blocks it, and the ports a project's live
    /// processes listen on are offered under their names.
    #[test]
    fn a_process_never_blocks_itself() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("api")]);
        harness.up();
        let sample = HashMap::from([(FAKE_LAUNCHER_PID, vec![
            ListeningSocket::builder()
                .port(PORT)
                .address(Ipv4Addr::LOCALHOST.into())
                .pid(FAKE_LAUNCHER_PID)
                .build(),
        ])]);
        harness.supervisor.take_ports(&sample);
        let held: HashMap<u16, PortHolder> = harness.supervisor.held_ports().into_iter().collect();
        assert_eq!(
            held[&PORT],
            PortHolder::builder()
                .project(test_project().name().clone())
                .process(name("api"))
                .build()
        );
        harness.supervisor.set_held_ports(held);

        harness
            .supervisor
            .restart(&name("api"), &LaunchEnvironment::default());

        assert!(
            harness.supervisor.snapshot().processes()[0]
                .blocked_on()
                .is_none()
        );
    }
}
