use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use super::entity::Supervisor;
use crate::{adapter::service::ProcessUsage, domain::value::PaneId};

/// How often each running process's resource usage is resampled.
pub(super) const METRICS_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

impl Supervisor {
    /// When usage is next resampled, if a sampler is wired and there is
    /// anything to sample. `None` when no process is live and no usage is
    /// cached, so an idle caller sleeps instead of waking every interval; a
    /// later spawn re-arms it.
    pub fn next_metrics_deadline(&self) -> Option<Instant> {
        self.process_metrics.as_ref()?;
        if self.live_pids().is_empty() && self.usage.is_empty() {
            return None;
        }
        Some(self.metrics_deadline)
    }

    /// Resamples every live process's resource usage once the metrics deadline
    /// has passed. Returns whether any usage changed.
    pub fn sample_metrics(&mut self, now: Instant) -> bool {
        if self.process_metrics.is_none() || now < self.metrics_deadline {
            return false;
        }
        self.metrics_deadline = now + METRICS_SAMPLE_INTERVAL;
        let live = self.live_pids();
        if live.is_empty() && self.usage.is_empty() {
            return false;
        }
        let pids: Vec<u32> = live.iter().map(|(_, pid)| *pid).collect();
        let samples = self
            .process_metrics
            .as_mut()
            .map(|metrics| metrics.sample(&pids))
            .unwrap_or_default();
        let updated: HashMap<PaneId, ProcessUsage> = live
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
        let changed = updated != self.usage;
        self.usage = updated;
        changed
    }

    /// The pane and process id of every live process that reports one.
    fn live_pids(&self) -> Vec<(PaneId, u32)> {
        self.panes
            .iter()
            .filter_map(|(pane, target)| {
                target
                    .handle
                    .as_ref()
                    .and_then(|handle| handle.process_id())
                    .map(|pid| (*pane, pid))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            port::{ProcessMetrics, ProcessSample},
            process::{Process, ProcessKind, RestartPolicy},
            pty::{ExitOutcome, ProcessOutput},
            value::{CommandLine, ProcessName},
        },
    };

    const PANE: u64 = 0;
    /// CPU load the fake sampler reports for every pid.
    const SAMPLED_CPU: f32 = 12.5;
    /// Memory the fake sampler reports for every pid.
    const SAMPLED_MEMORY: u64 = 4096;

    /// A sampler that counts its calls and reports a fixed load for every pid.
    struct FakeMetrics {
        calls: Rc<RefCell<usize>>,
    }

    impl ProcessMetrics for FakeMetrics {
        fn sample(&mut self, pids: &[u32]) -> HashMap<u32, ProcessSample> {
            *self.calls.borrow_mut() += 1;
            pids.iter()
                .map(|pid| {
                    (
                        *pid,
                        ProcessSample::builder()
                            .cpu_percent(SAMPLED_CPU)
                            .memory_bytes(SAMPLED_MEMORY)
                            .build(),
                    )
                })
                .collect()
        }
    }

    /// A supervisor over one terminal, with a counting sampler wired.
    fn sampled(processes: Vec<Process>) -> (Harness, Rc<RefCell<usize>>) {
        let mut harness = Harness::new(Box::new(PidRunner), processes);
        let calls = Rc::new(RefCell::new(0));
        harness
            .supervisor
            .set_process_metrics(Box::new(FakeMetrics {
                calls: calls.clone(),
            }));
        (harness, calls)
    }

    /// One terminal on [`PANE`].
    fn terminal() -> Process {
        Process::builder()
            .id(PaneId::new(PANE))
            .name(ProcessName::try_new("p").unwrap())
            .kind(ProcessKind::Terminal)
            .command(Some(CommandLine::try_new("sh").unwrap()))
            .restart(RestartPolicy::Never)
            .build()
    }

    /// Usage is resampled only once the metrics deadline has passed, and then
    /// reaches the snapshot.
    #[test]
    fn metrics_are_sampled_only_after_the_deadline() {
        let (mut harness, calls) = sampled(vec![terminal()]);
        harness.supervisor.up();
        let deadline = harness
            .supervisor
            .next_metrics_deadline()
            .expect("a live process arms the sampler");

        assert!(
            !harness
                .supervisor
                .sample_metrics(deadline - Duration::from_millis(1))
        );
        assert_eq!(*calls.borrow(), 0);
        assert!(harness.supervisor.sample_metrics(deadline));
        assert_eq!(
            *calls.borrow(),
            1,
            "sampling happens once the deadline passes"
        );

        let usage = harness.supervisor.snapshot().processes()[0]
            .usage()
            .expect("the sample reaches the snapshot");
        assert_eq!(*usage.pid(), FAKE_LAUNCHER_PID);
        assert_eq!(*usage.memory_bytes(), SAMPLED_MEMORY);
    }

    /// With no live process and no cached usage there is nothing to sample, so
    /// the timer is disarmed and the caller sleeps instead of waking every
    /// interval to no-op.
    #[test]
    fn metrics_timer_is_disarmed_when_nothing_is_live() {
        let (harness, _) = sampled(Vec::new());

        assert!(
            harness.supervisor.next_metrics_deadline().is_none(),
            "an idle project with nothing to sample disarms the metrics timer"
        );
    }

    /// The exit of the last live process drops its sampled usage at once, so
    /// the timer disarms without waiting for another sample.
    #[test]
    fn a_stopped_process_disarms_the_timer_once_its_usage_is_gone() {
        let (mut harness, _) = sampled(vec![terminal()]);
        harness.supervisor.up();
        let deadline = harness.supervisor.next_metrics_deadline().unwrap();
        harness.supervisor.sample_metrics(deadline);

        harness
            .supervisor
            .force_kill(&ProcessName::try_new("p").unwrap());
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert!(harness.supervisor.next_metrics_deadline().is_none());
    }
}
