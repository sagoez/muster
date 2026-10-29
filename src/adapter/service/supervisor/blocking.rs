//! Refusing a start whose port another process already listens on, so a
//! second dev server does not fail on `EADDRINUSE` while the first one keeps
//! serving.

use std::collections::HashMap;

use super::entity::Supervisor;
use crate::{
    adapter::service::{EventKind, NoticeLevel, PortBlock, PortHolder, PortOwner},
    domain::{process::ProcessState, value::PaneId},
};

/// Opens the notice of a start refused for a held port, before the port.
const BLOCKED_NOTICE: &str = "did not start: port";
/// Between the port and who holds it in that notice.
const HELD_BY: &str = "is held by";

impl Supervisor {
    /// The ports this project's live processes listen on, each with who
    /// holds it.
    pub fn held_ports(&self) -> Vec<(u16, PortHolder)> {
        self.metrics
            .listening()
            .filter_map(|(pane, sockets)| {
                let process = self.workspace.process(pane)?;
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
        self.metrics.set_held_ports(held);
    }

    /// Refuses to launch `pane` when another process listens on the port its
    /// readiness check names, whether a supervised process of any project or
    /// one muster does not run: the refusal is an error notice, a `blocked`
    /// event, the pane's `blocked_on` and its `blocked` state. Returns
    /// whether it refused.
    pub(super) fn refuse_blocked_launch(&mut self, pane: PaneId) -> bool {
        let Some(block) = self.port_block(pane) else {
            self.metrics.clear_block(pane);
            return false;
        };
        let message = format!(
            "{BLOCKED_NOTICE} {} {HELD_BY} {}",
            block.port(),
            block.holder()
        );
        self.notify(NoticeLevel::Error, Some(pane), message.clone());
        self.record_event(Some(pane), EventKind::Blocked, message);
        self.metrics.set_block(pane, block);
        self.workspace.set_state(pane, ProcessState::Blocked);
        true
    }

    /// What blocks `pane`'s launch: the port its readiness check names,
    /// held by another supervised process as last sampled, or else by any
    /// process the OS socket table shows listening on it. A port its own
    /// process was last seen on is free to it.
    fn port_block(&mut self, pane: PaneId) -> Option<PortBlock> {
        let process = self.workspace.process(pane)?;
        let port = process.ready().as_ref()?.probe().port()?;
        let me = PortHolder::builder()
            .project(self.project.name().clone())
            .process(process.name().clone())
            .build();
        let owner = match self.metrics.holder_of(port.into_inner()) {
            Some(holder) if *holder == me => return None,
            Some(holder) => PortOwner::Supervised(holder.clone()),
            None => PortOwner::Other(self.metrics.occupant(port.into_inner())?),
        };
        Some(PortBlock::builder().port(port).holder(owner).build())
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
            port::{ListeningSocket, PortOccupant, PortOccupants},
            process::{ReadyCheck, ReadyProbe, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::{LocalPort, Pid, PositiveDuration, ProjectName},
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

    /// The pid of the program muster does not run.
    const OUTSIDER_PID: u32 = 777;
    /// Its command line.
    const OUTSIDER_COMMAND: &str = "python3 -m http.server 3000";

    /// Answers every port lookup with the same occupant.
    struct Occupied(Option<PortOccupant>);

    impl PortOccupants for Occupied {
        fn occupant(&mut self, _port: u16) -> Option<PortOccupant> {
            self.0.clone()
        }
    }

    /// A program muster does not run, listening on [`PORT`].
    fn outsider() -> PortOccupant {
        PortOccupant::builder()
            .pid(Pid::try_new(OUTSIDER_PID).unwrap())
            .command(OUTSIDER_COMMAND.to_string())
            .build()
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
        assert_eq!(*web.state(), ProcessState::Blocked);
        let block = web.blocked_on().as_ref().expect("blocked");
        assert_eq!(
            block.holder(),
            &PortOwner::Supervised(holder("admin", "web"))
        );
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

    /// A port a process muster does not run listens on refuses the start too,
    /// naming that process by pid and command line.
    #[test]
    fn a_start_on_a_port_another_program_holds_is_refused_naming_it() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("web")]);
        harness
            .supervisor
            .metrics
            .set_port_occupants(Box::new(Occupied(Some(outsider()))));

        let batch = harness.supervisor.up(&LaunchEnvironment::default());

        assert!(batch.acted().is_empty());
        assert_eq!(
            batch.refused()[0].reason(),
            &format!("did not start: port 3000 is held by pid {OUTSIDER_PID} ({OUTSIDER_COMMAND})")
        );
        let status = harness.supervisor.snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Blocked);
        assert_eq!(
            status.processes()[0]
                .blocked_on()
                .as_ref()
                .map(PortBlock::holder),
            Some(&PortOwner::Other(outsider()))
        );
    }

    /// A policy respawn whose port was taken meanwhile is refused like a
    /// start: the process is blocked and nothing is launched.
    #[test]
    fn a_policy_respawn_on_a_taken_port_is_blocked() {
        let respawning = on_port("web").with_restart(RestartPolicy::Always);
        let mut harness = Harness::over(Box::new(PidRunner), &[respawning]);
        harness.up();
        let pane = harness.supervisor.pane_named(&name("web")).unwrap();
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].state(),
            ProcessState::Restarting
        );

        harness
            .supervisor
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));
        let respawn = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_respawn(pane, respawn);

        let status = harness.supervisor.snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Blocked);
        assert!(status.processes()[0].blocked_on().is_some());
        assert_eq!(harness.supervisor.generations()[&pane], respawn);
    }

    /// A process's own port never blocks it, and the ports a project's live
    /// processes listen on are offered under their names.
    #[test]
    fn a_process_never_blocks_itself() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("api")]);
        harness.up();
        let sample = HashMap::from([(fake_launcher_pid(), vec![
            ListeningSocket::builder()
                .port(PORT)
                .address(Ipv4Addr::LOCALHOST.into())
                .pid(fake_launcher_pid())
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
