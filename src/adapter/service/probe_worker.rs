use std::{
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};

use super::{
    event::ServiceEvent, probe_command::ProbeCommand, probe_target::ProbeTarget,
    probe_task::ProbeTask, supervisor::SpawnGeneration,
};
use crate::domain::value::{PaneId, ProjectRoot};

/// How often each pending probe is tried.
pub const PROBE_INTERVAL: Duration = Duration::from_millis(250);

/// Tries port and HTTP probes on its own thread, never the loop's, and posts
/// each pass back as [`ServiceEvent::ProbePassed`]. The thread ends once
/// every handle to it is dropped.
#[derive(Clone)]
pub struct ProbeWorker {
    /// Where the loop sends probes to try and forget.
    commands: Sender<ProbeCommand>,
}

impl ProbeWorker {
    /// Starts the worker, posting passes to `events`.
    pub fn spawn(events: Sender<ServiceEvent>) -> Self {
        let (commands, received) = unbounded();
        thread::spawn(move || run(&received, &events));
        Self { commands }
    }

    /// Tries `target` for the run `generation` of `pane` until it passes or
    /// `until`.
    pub fn watch(
        &self,
        project: &ProjectRoot,
        pane: PaneId,
        generation: SpawnGeneration,
        target: ProbeTarget,
        until: Instant,
    ) {
        let _ = self.commands.send(ProbeCommand::Watch(ProbeTask {
            project: project.clone(),
            pane,
            generation,
            target,
            until,
        }));
    }

    /// Gives up every probe of `pane`.
    pub fn forget(&self, project: &ProjectRoot, pane: PaneId) {
        let _ = self.commands.send(ProbeCommand::Forget {
            project: project.clone(),
            pane,
        });
    }
}

/// The worker's loop: takes commands, tries every pending probe each
/// [`PROBE_INTERVAL`], and posts each pass.
fn run(commands: &Receiver<ProbeCommand>, events: &Sender<ServiceEvent>) {
    let mut tasks: Vec<ProbeTask> = Vec::new();
    loop {
        let first = if tasks.is_empty() {
            commands.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            commands.recv_timeout(PROBE_INTERVAL)
        };
        match first {
            Ok(command) => apply(&mut tasks, command),
            Err(RecvTimeoutError::Timeout) => {},
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(command) = commands.try_recv() {
            apply(&mut tasks, command);
        }
        let now = Instant::now();
        tasks.retain(|task| task.until > now);
        let mut passed = Vec::new();
        tasks.retain(|task| {
            if task.target.passes() {
                passed.push((
                    task.project.clone(),
                    task.pane,
                    task.generation,
                    task.target.clone(),
                ));
                false
            } else {
                true
            }
        });
        for (project, pane, generation, probe) in passed {
            if events
                .send(ServiceEvent::ProbePassed {
                    project,
                    pane,
                    generation,
                    probe,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// Applies one command to the pending `tasks`; a probe already pending for
/// the same run is not added twice.
fn apply(tasks: &mut Vec<ProbeTask>, command: ProbeCommand) {
    match command {
        ProbeCommand::Watch(task) => {
            match tasks.iter_mut().find(|pending| {
                pending.project == task.project
                    && pending.pane == task.pane
                    && pending.generation == task.generation
                    && pending.target == task.target
            }) {
                Some(pending) => pending.until = pending.until.max(task.until),
                None => tasks.push(task),
            }
        },
        ProbeCommand::Forget { project, pane } => {
            tasks.retain(|task| task.project != project || task.pane != pane);
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, TcpListener},
        path::PathBuf,
    };

    use crossbeam_channel::bounded;

    use super::*;
    use crate::domain::value::LocalPort;

    /// Longest a test waits for a probe to pass.
    const PASS_WAIT: Duration = Duration::from_secs(5);

    fn root() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/shop")).unwrap()
    }

    #[test]
    fn a_listening_port_passes_once() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = LocalPort::try_new(listener.local_addr().unwrap().port()).unwrap();
        let (sender, events) = bounded(8);
        let worker = ProbeWorker::spawn(sender);
        let generation = SpawnGeneration::initial();
        let until = Instant::now() + PASS_WAIT;

        worker.watch(
            &root(),
            PaneId::new(1),
            generation,
            ProbeTarget::Port(port),
            until,
        );

        match events.recv_timeout(PASS_WAIT).unwrap() {
            ServiceEvent::ProbePassed { pane, probe, .. } => {
                assert_eq!(pane, PaneId::new(1));
                assert_eq!(probe, ProbeTarget::Port(port));
            },
            _ => panic!("expected a pass"),
        }
        assert!(events.recv_timeout(PROBE_INTERVAL * 4).is_err());
    }

    #[test]
    fn a_probe_is_pending_once_per_run_until_its_pane_is_forgotten() {
        let port = LocalPort::try_new(1).unwrap();
        let task = |pane: u64, generation: SpawnGeneration| {
            ProbeCommand::Watch(ProbeTask {
                project: root(),
                pane: PaneId::new(pane),
                generation,
                target: ProbeTarget::Port(port),
                until: Instant::now() + PASS_WAIT,
            })
        };
        let first = SpawnGeneration::initial();
        let mut tasks = Vec::new();

        apply(&mut tasks, task(1, first));
        apply(&mut tasks, task(1, first));
        apply(&mut tasks, task(1, first.next()));
        apply(&mut tasks, task(2, first));
        assert_eq!(tasks.len(), 3);

        apply(&mut tasks, ProbeCommand::Forget {
            project: root(),
            pane: PaneId::new(1),
        });
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].pane, PaneId::new(2));
    }
}
