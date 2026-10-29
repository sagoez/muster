//! Refusing a start whose port another process already listens on, so a
//! second dev server does not fail on `EADDRINUSE` while the first one keeps
//! serving.

use std::collections::HashMap;

use super::{entity::Supervisor, spawn_generation::SpawnGeneration};
use crate::{
    adapter::service::{EventKind, NoticeLevel, PortBlock, PortHolder, PortOwner, WaitStatus},
    domain::{
        port::PortOccupant,
        process::ProcessState,
        value::{LocalPort, PaneId, Pid},
    },
};

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

    /// Refuses to launch `pane` when a supervised process of any project was
    /// last sampled on the port its readiness check names, as
    /// [`Self::block_launch`] does. A port held by a process muster does not
    /// run is found by the launch worker instead, since that lookup scans
    /// the whole socket table. Returns whether it refused.
    pub(super) fn refuse_blocked_launch(&mut self, pane: PaneId) -> bool {
        let Some(block) = self.sampled_block(pane) else {
            self.metrics.clear_block(pane);
            return false;
        };
        self.block_launch(pane, block);
        true
    }

    /// Refuses `pane`'s launch for `block`: an error notice, a `blocked`
    /// event, the pane's `blocked_on` and its `blocked` state.
    fn block_launch(&mut self, pane: PaneId, block: PortBlock) {
        let message = block.to_string();
        self.notify(NoticeLevel::Error, Some(pane), message.clone());
        self.record_event(Some(pane), EventKind::Blocked, message);
        self.metrics.set_block(pane, block);
        self.workspace.set_state(pane, ProcessState::Blocked);
    }

    /// What blocks `pane`'s launch as last sampled: the port its readiness
    /// check names, held by another supervised process. A port its own
    /// process was last seen on is free to it.
    fn sampled_block(&self, pane: PaneId) -> Option<PortBlock> {
        let process = self.workspace.process(pane)?;
        let port = self.launch_port(pane)?;
        let me = PortHolder::builder()
            .project(self.project.name().clone())
            .process(process.name().clone())
            .build();
        let holder = self
            .metrics
            .holder_of(port.into_inner())
            .filter(|holder| **holder != me)?;
        Some(
            PortBlock::builder()
                .port(port)
                .holder(PortOwner::Supervised(holder.clone()))
                .build(),
        )
    }

    /// The port `pane`'s readiness check names, which its launch needs free.
    pub(super) fn launch_port(&self, pane: PaneId) -> Option<LocalPort> {
        self.workspace
            .process(pane)?
            .ready()
            .as_ref()?
            .probe()
            .port()
    }

    /// Who runs a live process of this project whose pid is among `pids`,
    /// as the holder of a port that process tree listens on.
    pub fn holder_of_pids(&self, pids: &[Pid]) -> Option<PortHolder> {
        self.live_pids()
            .into_iter()
            .find(|(_, pid)| pids.contains(pid))
            .and_then(|(pane, _)| self.workspace.process(pane))
            .map(|process| {
                PortHolder::builder()
                    .project(self.project.name().clone())
                    .process(process.name().clone())
                    .build()
            })
    }

    /// Settles the launch of `pane` at `generation`, whose worker found the
    /// port it needs held as `block` says, so nothing was launched: the
    /// launch is refused as [`Self::block_launch`] does, its waits end and
    /// what was asked meanwhile applies. A launch a later action superseded
    /// is only forgotten.
    pub fn handle_launch_blocked(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        block: PortBlock,
    ) {
        let Some(launch) = self.take_launch(pane, generation) else {
            self.stale_release(pane, generation);
            return;
        };
        let (membership, intent) = (launch.membership(), launch.intent());
        self.readiness.remove(&pane);
        self.block_launch(pane, block);
        self.end_waits(
            pane,
            Some(generation),
            WaitStatus::NotRunning,
            ProcessState::Blocked,
        );
        self.apply_pending(pane, membership, None, intent);
    }

    /// The owner of the port a launch of `pane` found `occupant` holding: the
    /// live process of this project it belongs to, else the occupant itself.
    pub(super) fn owner_of(&self, occupant: PortOccupant) -> PortOwner {
        self.holder_of_pids(&occupant.lineage())
            .map_or(PortOwner::Other(occupant), PortOwner::Supervised)
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use super::*;
    use crate::{
        adapter::service::{LaunchEnvironment, ServiceEvent, supervisor::testing::*},
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

    /// A launch on a worker asks who holds its port there, never on the
    /// loop: the start is `starting` until the worker's answer settles it
    /// blocked, with its wait ended, and nothing is launched.
    #[test]
    fn a_worker_launch_finds_its_port_held_off_the_loop() {
        let mut harness = Harness::over(Box::new(WorkerRunner::default()), &[on_port("web")]);
        harness
            .supervisor
            .metrics
            .set_port_occupants(Box::new(Occupied(Some(outsider()))));

        harness
            .supervisor
            .start(&name("web"), &LaunchEnvironment::default());
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].state(),
            ProcessState::Starting
        );
        let Ok(ServiceEvent::LaunchBlocked {
            pane,
            generation,
            port,
            occupant,
            ..
        }) = harness.events.recv_timeout(Duration::from_secs(5))
        else {
            panic!("the worker reports the held port");
        };
        let block = PortBlock::builder()
            .port(port)
            .holder(harness.supervisor.owner_of(occupant))
            .build();
        harness
            .supervisor
            .handle_launch_blocked(pane, generation, block);

        let status = harness.supervisor.snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Blocked);
        assert_eq!(
            status.processes()[0]
                .blocked_on()
                .as_ref()
                .map(PortBlock::holder),
            Some(&PortOwner::Other(outsider()))
        );
        assert!(harness.supervisor.launches_in_flight() == 0);
    }

    /// A port a supervised process's child listens on is named after that
    /// process even before any port sample, from the occupant's ancestors.
    #[test]
    fn a_port_a_supervised_tree_holds_is_named_before_any_sample() {
        let mut harness = Harness::over(Box::new(PidRunner), &[
            on_port("web"),
            spec("api", "true", RestartPolicy::Never),
        ]);
        harness
            .supervisor
            .start(&name("api"), &LaunchEnvironment::default());
        let child = PortOccupant::builder()
            .pid(Pid::try_new(OUTSIDER_PID).unwrap())
            .command(OUTSIDER_COMMAND.to_string())
            .ancestors(vec![fake_launcher_pid()])
            .build();
        harness
            .supervisor
            .metrics
            .set_port_occupants(Box::new(Occupied(Some(child))));

        harness
            .supervisor
            .start(&name("web"), &LaunchEnvironment::default());

        let status = harness.supervisor.snapshot();
        let web = status
            .processes()
            .iter()
            .find(|process| process.name() == &name("web"))
            .unwrap();
        assert_eq!(*web.state(), ProcessState::Blocked);
        assert_eq!(
            web.blocked_on().as_ref().map(PortBlock::holder),
            Some(&PortOwner::Supervised(
                PortHolder::builder()
                    .project(test_project().name().clone())
                    .process(name("api"))
                    .build()
            ))
        );
    }
}
