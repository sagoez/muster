//! Refusing a start whose port another process already listens on, so a
//! second dev server does not fail on `EADDRINUSE` while the first one keeps
//! serving.

use std::collections::HashMap;

use super::{entity::Supervisor, spawn_generation::SpawnGeneration};
use crate::{
    adapter::service::{EventKind, NoticeLevel, PortBlock, PortHolder, PortOwner, WaitStatus},
    domain::{
        port::PortOccupant,
        process::{Process, ProcessState},
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
    /// on, which a start checks the ports it needs against.
    pub fn set_held_ports(&mut self, held: HashMap<u16, PortHolder>) {
        self.metrics.set_held_ports(held);
    }

    /// Refuses to launch `pane` when a supervised process of any project was
    /// last sampled on a port it needs, as [`Self::block_launch`] does. A
    /// port held by a process muster does not run is found by the launch
    /// worker instead, since that lookup scans the whole socket table.
    /// Returns whether it refused.
    pub(super) fn refuse_blocked_launch(&mut self, pane: PaneId) -> bool {
        let Some(block) = self.sampled_block(pane) else {
            self.metrics.clear_block(pane);
            return false;
        };
        self.block_launch(pane, block);
        true
    }

    /// Refuses `pane`'s launch for `block`: an error notice, a refusal
    /// carrying the block, a `blocked` event, the pane's `blocked_on` and its
    /// `blocked` state.
    fn block_launch(&mut self, pane: PaneId, block: PortBlock) {
        let message = block.to_string();
        match self.workspace.process(pane) {
            Some(process) => {
                self.attention
                    .notify_blocked(self.root(), process.name().clone(), block.clone());
            },
            None => self.notify(NoticeLevel::Error, None, message.clone()),
        }
        self.record_event(Some(pane), EventKind::Blocked, message);
        self.metrics.set_block(pane, block);
        self.workspace.set_state(pane, ProcessState::Blocked);
    }

    /// What blocks `pane`'s launch as last sampled: the first port it needs
    /// that another supervised process holds. A port its own process was last
    /// seen on is free to it. When a lower port it needs could still be held
    /// by a program muster does not run, the launch worker's scan decides
    /// instead, so the first held port is the one named.
    fn sampled_block(&self, pane: PaneId) -> Option<PortBlock> {
        let process = self.workspace.process(pane)?;
        let me = PortHolder::builder()
            .project(self.project.name().clone())
            .process(process.name().clone())
            .build();
        let (lower, port, holder) =
            self.launch_ports(pane)
                .into_iter()
                .enumerate()
                .find_map(|(lower, port)| {
                    let holder = self
                        .metrics
                        .holder_of(port.into_inner())
                        .filter(|holder| **holder != me)?;
                    Some((lower, port, holder))
                })?;
        if lower > 0 && self.metrics.port_occupants().is_some() {
            return None;
        }
        Some(
            PortBlock::builder()
                .port(port)
                .holder(PortOwner::Supervised(holder.clone()))
                .build(),
        )
    }

    /// The ports `pane`'s launch needs free, ascending: those its process
    /// declares and the one its readiness check probes, each once.
    pub(super) fn launch_ports(&self, pane: PaneId) -> Vec<LocalPort> {
        self.workspace
            .process(pane)
            .map(Process::launch_ports)
            .unwrap_or_default()
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
    /// launch is refused as `Self::block_launch` does, its waits end and
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
    use std::{net::Ipv4Addr, sync::PoisonError, time::Duration};

    use super::*;
    use crate::{
        adapter::service::{LaunchEnvironment, ServiceEvent, supervisor::testing::*},
        domain::{
            config::{DeclaredPorts, ProcessSpec},
            port::{ListeningSocket, PortOccupant, ProcessRunner},
            process::{ReadyCheck, ReadyProbe, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::{LocalPort, Pid, PositiveDuration, ProjectName},
        },
    };

    /// The port every process here declares.
    const PORT: u16 = 3000;
    /// A port between [`PORT`] and [`SECOND_PORT`].
    const BETWEEN_PORT: u16 = 4000;
    /// A second declared port, above [`PORT`].
    const SECOND_PORT: u16 = 5173;

    /// `raw` as a port.
    fn port(raw: u16) -> LocalPort {
        LocalPort::try_new(raw).unwrap()
    }

    /// `raw`, checking port [`PORT`] for readiness.
    fn on_port(raw: &str) -> ProcessSpec {
        spec(raw, "true", RestartPolicy::Never).with_ready(Some(
            ReadyCheck::builder()
                .probe(ReadyProbe::Port(port(PORT)))
                .timeout(PositiveDuration::try_new(Duration::from_secs(30)).unwrap())
                .build(),
        ))
    }

    /// `raw`, declaring `ports` and no readiness check.
    fn declaring(raw: &str, ports: &[u16]) -> ProcessSpec {
        spec(raw, "true", RestartPolicy::Never)
            .with_ports(DeclaredPorts::of(ports.iter().copied().map(port)).unwrap())
    }

    /// `port` held by `holder`, as a refusal carries it.
    fn supervised_block(at: u16, holder: PortHolder) -> PortBlock {
        PortBlock::builder()
            .port(port(at))
            .holder(PortOwner::Supervised(holder))
            .build()
    }

    /// The ports of every scan `asked` records.
    fn scans(asked: &AskedScans) -> Vec<Vec<LocalPort>> {
        asked.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// How many `blocked` events `harness` recorded.
    fn blocked_events(harness: &Harness) -> usize {
        harness
            .event_log()
            .lock()
            .unwrap()
            .page(harness.supervisor().root(), None, &[EventKind::Blocked])
            .events()
            .len()
    }

    /// The pid of the program muster does not run.
    const OUTSIDER_PID: u32 = 777;
    /// Its command line.
    const OUTSIDER_COMMAND: &str = "python3 -m http.server 3000";

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
            .supervisor_mut()
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert!(batch.acted().is_empty());
        assert_eq!(
            batch.refused()[0].reason(),
            "did not start: port 3000 is held by project 'admin', process 'web'"
        );
        let status = harness.supervisor_mut().snapshot();
        let web = &status.processes()[0];
        assert_eq!(*web.state(), ProcessState::Blocked);
        let block = web.blocked_on().as_ref().expect("blocked");
        assert_eq!(
            block.holder(),
            &PortOwner::Supervised(holder("admin", "web"))
        );
        assert_eq!(
            harness
                .event_log()
                .lock()
                .unwrap()
                .page(harness.supervisor().root(), None, &[EventKind::Blocked])
                .events()
                .len(),
            1
        );

        harness.supervisor_mut().set_held_ports(HashMap::new());
        harness.supervisor_mut().up(&LaunchEnvironment::default());
        let status = harness.supervisor_mut().snapshot();
        assert!(status.processes()[0].blocked_on().is_none());
        assert!(status.processes()[0].state().may_be_running());
    }

    /// A port a process muster does not run listens on refuses the start too,
    /// naming that process by pid and command line.
    #[test]
    fn a_start_on_a_port_another_program_holds_is_refused_naming_it() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("web")]);
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(Occupied::by(Some(outsider()))));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert!(batch.acted().is_empty());
        assert_eq!(
            batch.refused()[0].reason(),
            &format!("did not start: port 3000 is held by pid {OUTSIDER_PID} ({OUTSIDER_COMMAND})")
        );
        let status = harness.supervisor_mut().snapshot();
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
        let pane = harness.supervisor_mut().pane_named(&name("web")).unwrap();
        let generation = harness.supervisor_mut().generations()[&pane];
        harness.supervisor_mut().handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert_eq!(
            *harness.supervisor_mut().snapshot().processes()[0].state(),
            ProcessState::Restarting
        );

        harness
            .supervisor_mut()
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));
        let respawn = harness.supervisor_mut().generations()[&pane];
        harness.supervisor_mut().handle_respawn(pane, respawn);

        let status = harness.supervisor_mut().snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Blocked);
        assert!(status.processes()[0].blocked_on().is_some());
        assert_eq!(harness.supervisor_mut().generations()[&pane], respawn);
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
        harness.supervisor_mut().take_ports(&sample);
        let held: HashMap<u16, PortHolder> =
            harness.supervisor_mut().held_ports().into_iter().collect();
        assert_eq!(
            held[&PORT],
            PortHolder::builder()
                .project(test_project().name().clone())
                .process(name("api"))
                .build()
        );
        harness.supervisor_mut().set_held_ports(held);

        harness
            .supervisor_mut()
            .restart(&name("api"), &LaunchEnvironment::default());

        assert!(
            harness.supervisor_mut().snapshot().processes()[0]
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
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(Occupied::by(Some(outsider()))));

        harness
            .supervisor_mut()
            .start(&name("web"), &LaunchEnvironment::default());
        assert_eq!(
            *harness.supervisor_mut().snapshot().processes()[0].state(),
            ProcessState::Starting
        );
        let Ok(ServiceEvent::LaunchBlocked {
            pane,
            generation,
            port,
            occupant,
            ..
        }) = harness.events().recv_timeout(Duration::from_secs(5))
        else {
            panic!("the worker reports the held port");
        };
        let block = PortBlock::builder()
            .port(port)
            .holder(harness.supervisor_mut().owner_of(occupant))
            .build();
        harness
            .supervisor_mut()
            .handle_launch_blocked(pane, generation, block);

        let status = harness.supervisor_mut().snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Blocked);
        assert_eq!(
            status.processes()[0]
                .blocked_on()
                .as_ref()
                .map(PortBlock::holder),
            Some(&PortOwner::Other(outsider()))
        );
        assert!(harness.supervisor_mut().launches_in_flight() == 0);
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
            .supervisor_mut()
            .start(&name("api"), &LaunchEnvironment::default());
        let child = PortOccupant::builder()
            .pid(Pid::try_new(OUTSIDER_PID).unwrap())
            .command(OUTSIDER_COMMAND.to_string())
            .ancestors(vec![fake_launcher_pid()])
            .build();
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(Occupied::by(Some(child))));

        harness
            .supervisor_mut()
            .start(&name("web"), &LaunchEnvironment::default());

        let status = harness.supervisor_mut().snapshot();
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

    /// Every declared port is checked, not only the readiness one: a held
    /// second port refuses the start, and of two held the lowest is named.
    #[test]
    fn every_declared_port_is_checked_and_the_first_held_blocks() {
        let mut harness = Harness::over(Box::new(PidRunner), &[declaring("web", &[
            PORT,
            SECOND_PORT,
        ])]);
        harness
            .supervisor_mut()
            .set_held_ports(HashMap::from([(SECOND_PORT, holder("admin", "web"))]));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert!(batch.acted().is_empty());
        assert_eq!(
            batch.refused()[0].block().as_ref(),
            Some(&supervised_block(SECOND_PORT, holder("admin", "web")))
        );

        harness.supervisor_mut().set_held_ports(HashMap::from([
            (PORT, holder("admin", "api")),
            (SECOND_PORT, holder("admin", "web")),
        ]));
        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert_eq!(
            batch.refused()[0].block().as_ref(),
            Some(&supervised_block(PORT, holder("admin", "api")))
        );
        assert_eq!(
            batch.refused()[0].reason(),
            "did not start: port 3000 is held by project 'admin', process 'api'"
        );
    }

    /// A port both declared and probed for readiness is asked about once, in
    /// one scan with the other declared ports, ascending.
    #[test]
    fn a_declared_port_and_the_readiness_port_are_checked_once() {
        let web =
            on_port("web").with_ports(DeclaredPorts::of([port(SECOND_PORT), port(PORT)]).unwrap());
        let mut harness = Harness::over(Box::new(PidRunner), &[web]);
        let occupied = Occupied::by(None);
        let asked = occupied.asked();
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(occupied));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert_eq!(batch.acted(), &[name("web")]);
        assert_eq!(scans(&asked), [vec![port(PORT), port(SECOND_PORT)]]);
    }

    /// A launch on a worker asks about every port it needs in one scan, and
    /// the first one held settles it blocked, naming that port.
    #[test]
    fn a_worker_launch_scans_the_socket_table_once_for_all_ports() {
        let web = on_port("web")
            .with_ports(DeclaredPorts::of([port(SECOND_PORT), port(BETWEEN_PORT)]).unwrap());
        let mut harness = Harness::over(Box::new(WorkerRunner::default()), &[web]);
        let occupied = Occupied::on(&[port(BETWEEN_PORT), port(SECOND_PORT)], outsider());
        let asked = occupied.asked();
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(occupied));

        harness
            .supervisor_mut()
            .start(&name("web"), &LaunchEnvironment::default());
        let Ok(ServiceEvent::LaunchBlocked {
            pane,
            generation,
            port: held,
            occupant,
            ..
        }) = harness.events().recv_timeout(Duration::from_secs(5))
        else {
            panic!("the worker reports the held port");
        };

        assert_eq!(held, port(BETWEEN_PORT));
        assert_eq!(occupant, outsider());
        assert_eq!(scans(&asked), [vec![
            port(PORT),
            port(BETWEEN_PORT),
            port(SECOND_PORT)
        ]]);
        let block = PortBlock::builder()
            .port(held)
            .holder(harness.supervisor_mut().owner_of(occupant))
            .build();
        harness
            .supervisor_mut()
            .handle_launch_blocked(pane, generation, block);
        assert_eq!(
            harness.supervisor_mut().snapshot().processes()[0]
                .blocked_on()
                .as_ref()
                .map(PortBlock::port),
            Some(port(BETWEEN_PORT))
        );
    }

    /// A refusal of `up` for a held port carries the block, whether a
    /// supervised process or another program holds it.
    #[test]
    fn an_up_refusal_carries_its_block() {
        let mut harness = Harness::over(Box::new(PidRunner), &[on_port("web")]);
        harness
            .supervisor_mut()
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert_eq!(
            batch.refused()[0].block().as_ref(),
            Some(&supervised_block(PORT, holder("admin", "web")))
        );

        harness.supervisor_mut().set_held_ports(HashMap::new());
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(Occupied::by(Some(outsider()))));
        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert_eq!(
            batch.refused()[0].block().as_ref(),
            Some(
                &PortBlock::builder()
                    .port(port(PORT))
                    .holder(PortOwner::Other(outsider()))
                    .build()
            )
        );
    }

    /// A process that declares no port and probes none starts without any
    /// scan of the socket table, even with every port held.
    #[test]
    fn a_process_with_no_ports_is_never_scanned() {
        for runner in [
            Box::new(PidRunner) as Box<dyn ProcessRunner>,
            Box::new(WorkerRunner::default()),
        ] {
            let mut harness = Harness::over(runner, &[spec("api", "true", RestartPolicy::Never)]);
            let occupied = Occupied::by(Some(outsider()));
            let asked = occupied.asked();
            harness
                .supervisor_mut()
                .metrics
                .set_port_occupants(Box::new(occupied));

            let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

            assert_eq!(batch.acted(), &[name("api")]);
            assert!(scans(&asked).is_empty());
        }
    }

    /// A supervised process sampled on a higher port does not hide a program
    /// muster does not run on a lower one: the launch goes to the scan, which
    /// names the lower port, and the start is refused once.
    #[test]
    fn an_outsider_on_a_lower_port_is_named_before_a_sampled_holder_of_a_higher_one() {
        let mut harness = Harness::over(Box::new(WorkerRunner::default()), &[declaring("web", &[
            PORT,
            SECOND_PORT,
        ])]);
        harness
            .supervisor_mut()
            .set_held_ports(HashMap::from([(SECOND_PORT, holder("admin", "web"))]));
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(Occupied::on(&[port(PORT)], outsider())));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());
        assert!(batch.refused().is_empty());
        let Ok(ServiceEvent::LaunchBlocked {
            pane,
            generation,
            port: held,
            occupant,
            ..
        }) = harness.events().recv_timeout(Duration::from_secs(5))
        else {
            panic!("the worker reports the held port");
        };
        let block = PortBlock::builder()
            .port(held)
            .holder(harness.supervisor_mut().owner_of(occupant))
            .build();
        harness
            .supervisor_mut()
            .handle_launch_blocked(pane, generation, block);

        let status = harness.supervisor_mut().snapshot();
        assert_eq!(
            status.processes()[0].blocked_on().as_ref(),
            Some(
                &PortBlock::builder()
                    .port(port(PORT))
                    .holder(PortOwner::Other(outsider()))
                    .build()
            )
        );
        assert_eq!(blocked_events(&harness), 1);
    }

    /// A supervised process sampled on the lowest port a launch needs refuses
    /// it at once, without scanning the socket table.
    #[test]
    fn a_sampled_holder_of_the_lowest_port_refuses_without_a_scan() {
        let mut harness = Harness::over(Box::new(PidRunner), &[declaring("web", &[
            PORT,
            SECOND_PORT,
        ])]);
        harness
            .supervisor_mut()
            .set_held_ports(HashMap::from([(PORT, holder("admin", "web"))]));
        let occupied = Occupied::by(Some(outsider()));
        let asked = occupied.asked();
        harness
            .supervisor_mut()
            .metrics
            .set_port_occupants(Box::new(occupied));

        let batch = harness.supervisor_mut().up(&LaunchEnvironment::default());

        assert_eq!(
            batch.refused()[0].block().as_ref(),
            Some(&supervised_block(PORT, holder("admin", "web")))
        );
        assert!(scans(&asked).is_empty());
        assert_eq!(blocked_events(&harness), 1);
    }
}
