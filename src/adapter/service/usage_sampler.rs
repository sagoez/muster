use std::time::{Duration, Instant};

use super::supervisor::Supervisor;
use crate::domain::{port::ProcessMetrics, value::Pid};

/// How often each running process's resource usage is resampled.
pub const USAGE_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// Samples the resource usage of every loaded project's processes in one
/// pass, so the service holds one sampler however many projects it runs.
pub struct UsageSampler {
    /// Reads usage for a set of process trees.
    metrics: Box<dyn ProcessMetrics>,
    /// When usage is next resampled.
    deadline: Instant,
}

impl UsageSampler {
    /// A sampler reading through `metrics`, first due one interval from now.
    pub fn new(metrics: Box<dyn ProcessMetrics>) -> Self {
        Self {
            metrics,
            deadline: Instant::now() + USAGE_SAMPLE_INTERVAL,
        }
    }

    /// When usage is next resampled, or `None` when no supervisor has a
    /// live process or cached usage, so an idle service sleeps instead of
    /// waking every interval; a later spawn re-arms it.
    pub fn next_deadline<'a>(
        &self,
        supervisors: impl IntoIterator<Item = &'a Supervisor>,
    ) -> Option<Instant> {
        supervisors
            .into_iter()
            .any(Supervisor::wants_usage)
            .then_some(self.deadline)
    }

    /// Resamples every supervisor's live processes in one read once the
    /// deadline has passed. Returns whether any usage changed.
    pub fn sample<'a>(
        &mut self,
        now: Instant,
        supervisors: impl IntoIterator<Item = &'a mut Supervisor>,
    ) -> bool {
        if now < self.deadline {
            return false;
        }
        self.deadline = now + USAGE_SAMPLE_INTERVAL;
        let sampled: Vec<&mut Supervisor> = supervisors
            .into_iter()
            .filter(|supervisor| supervisor.wants_usage())
            .collect();
        if sampled.is_empty() {
            return false;
        }
        let pids: Vec<Pid> = sampled
            .iter()
            .flat_map(|supervisor| supervisor.usage_pids())
            .collect();
        let samples = self.metrics.sample(&pids);
        let mut changed = false;
        for supervisor in sampled {
            changed |= supervisor.take_usage(&samples);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use super::*;
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            port::ProcessSample,
            process::{Process, RestartPolicy},
            value::PaneId,
        },
    };

    /// Memory the fake sampler reports for every pid.
    const SAMPLED_MEMORY: u64 = 4096;

    /// A sampler that records the pids of every call and reports a fixed load
    /// for each.
    struct FakeMetrics {
        calls: Rc<RefCell<Vec<Vec<Pid>>>>,
    }

    impl ProcessMetrics for FakeMetrics {
        fn sample(&mut self, pids: &[Pid]) -> HashMap<Pid, ProcessSample> {
            self.calls.borrow_mut().push(pids.to_vec());
            pids.iter()
                .map(|pid| {
                    (
                        *pid,
                        ProcessSample::builder()
                            .cpu_percent(0.0)
                            .memory_bytes(SAMPLED_MEMORY)
                            .build(),
                    )
                })
                .collect()
        }
    }

    /// A sampler over [`FakeMetrics`] and the calls it records.
    fn sampler() -> (UsageSampler, Rc<RefCell<Vec<Vec<Pid>>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let sampler = UsageSampler::new(Box::new(FakeMetrics {
            calls: calls.clone(),
        }));
        (sampler, calls)
    }

    /// A project running one process `p`.
    fn running() -> Harness {
        let process: Process = spec("p", "sh", RestartPolicy::Never).to_process(PaneId::new(0));
        let mut harness = Harness::new(Box::new(PidRunner), vec![process]);
        harness.up();
        harness
    }

    /// Two projects are sampled in one read, only once the deadline passed,
    /// and each takes its own usage.
    #[test]
    fn every_project_is_sampled_in_one_read_once_due() {
        let (mut sampler, calls) = sampler();
        let mut first = running();
        let mut second = running();
        let deadline = sampler
            .next_deadline([&first.supervisor, &second.supervisor])
            .expect("a live process arms the sampler");

        assert!(!sampler.sample(deadline - Duration::from_millis(1), [
            &mut first.supervisor,
            &mut second.supervisor
        ],));
        assert!(calls.borrow().is_empty());
        assert!(sampler.sample(deadline, [&mut first.supervisor, &mut second.supervisor]));

        assert_eq!(*calls.borrow(), vec![vec![
            fake_launcher_pid(),
            fake_launcher_pid()
        ]]);
        for harness in [&first, &second] {
            let usage = harness.supervisor.snapshot().processes()[0]
                .usage()
                .expect("each project takes the sample");
            assert_eq!(*usage.memory_bytes(), SAMPLED_MEMORY);
        }
    }

    /// With nothing live and no usage cached anywhere, the timer is disarmed.
    #[test]
    fn the_timer_is_disarmed_when_nothing_is_live() {
        let (sampler, _) = sampler();
        let idle = Harness::new(Box::new(PidRunner), Vec::new());

        assert!(sampler.next_deadline([&idle.supervisor]).is_none());
    }
}
