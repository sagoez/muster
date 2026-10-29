use std::collections::HashMap;

use super::entity::Supervisor;
use crate::{
    adapter::service::ProcessUsage,
    domain::{
        port::ProcessSample,
        value::{PaneId, Pid},
    },
};

impl Supervisor {
    /// Whether a usage sample means anything here: a process is live, or
    /// usage is still cached for one that is no longer sampled.
    pub fn wants_usage(&self) -> bool {
        !self.live_pids().is_empty() || self.metrics.has_usage()
    }

    /// The pid of every live process whose usage is sampled.
    pub fn usage_pids(&self) -> Vec<Pid> {
        self.live_pids().into_iter().map(|(_, pid)| pid).collect()
    }

    /// Takes each live process's usage from `samples`, keyed by pid, dropping
    /// usage for a process the samples no longer find. Returns whether any
    /// usage changed.
    pub fn take_usage(&mut self, samples: &HashMap<Pid, ProcessSample>) -> bool {
        let updated: HashMap<PaneId, ProcessUsage> = self
            .live_pids()
            .into_iter()
            .filter_map(|(pane, pid)| {
                samples.get(&pid).map(|sample| {
                    (
                        pane,
                        ProcessUsage::builder()
                            .pid(pid)
                            .cpu_percent(*sample.cpu_percent())
                            .memory_bytes(*sample.memory_bytes())
                            .build(),
                    )
                })
            })
            .collect();
        self.metrics.replace_usage(updated)
    }

    /// The pane and process id of every live process that reports one.
    pub(super) fn live_pids(&self) -> Vec<(PaneId, Pid)> {
        self.panes
            .iter()
            .filter_map(|(pane, target)| {
                target
                    .handle()
                    .as_ref()
                    .and_then(|handle| handle.process_id())
                    .map(|pid| (*pane, pid))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            process::{Process, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
        },
    };

    const PANE: u64 = 0;
    /// CPU load the sample reports.
    const SAMPLED_CPU: f32 = 12.5;
    /// Memory the sample reports.
    const SAMPLED_MEMORY: u64 = 4096;

    /// One process `p` on [`PANE`].
    fn terminal() -> Process {
        spec("p", "sh", RestartPolicy::Never).to_process(PaneId::new(PANE))
    }

    /// A sample of [`SAMPLED_CPU`] and [`SAMPLED_MEMORY`] for `pid`.
    fn sample_of(pid: Pid) -> HashMap<Pid, ProcessSample> {
        HashMap::from([(
            pid,
            ProcessSample::builder()
                .cpu_percent(SAMPLED_CPU)
                .memory_bytes(SAMPLED_MEMORY)
                .build(),
        )])
    }

    /// A live process's sample reaches the snapshot, keyed by its pid.
    #[test]
    fn a_sample_reaches_the_snapshot() {
        let mut harness = Harness::new(Box::new(PidRunner), vec![terminal()]);
        harness.up();

        assert_eq!(harness.supervisor.usage_pids(), vec![fake_launcher_pid()]);
        assert!(
            harness
                .supervisor
                .take_usage(&sample_of(fake_launcher_pid()))
        );

        let usage = harness.supervisor.snapshot().processes()[0]
            .usage()
            .expect("the sample reaches the snapshot");
        assert_eq!(*usage.pid(), fake_launcher_pid());
        assert_eq!(*usage.memory_bytes(), SAMPLED_MEMORY);
    }

    /// With no live process and no cached usage, a sample means nothing.
    #[test]
    fn an_idle_project_wants_no_usage() {
        let harness = Harness::new(Box::new(PidRunner), Vec::new());

        assert!(!harness.supervisor.wants_usage());
    }

    /// The exit of the last live process drops its sampled usage at once, so
    /// the project stops wanting samples without waiting for another one.
    #[test]
    fn a_stopped_process_drops_its_usage_at_once() {
        let mut harness = Harness::new(Box::new(PidRunner), vec![terminal()]);
        harness.up();
        harness
            .supervisor
            .take_usage(&sample_of(fake_launcher_pid()));

        harness.supervisor.force_kill(&name("p"));
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert!(!harness.supervisor.wants_usage());
    }
}
