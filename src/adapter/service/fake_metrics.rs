use std::collections::HashMap;

use super::sampled_calls::SampledCalls;
use crate::domain::{
    port::{ProcessMetrics, ProcessSample},
    value::Pid,
};

/// Memory the fake sampler reports for every pid.
pub(super) const SAMPLED_MEMORY: u64 = 4096;

/// A sampler that records the pids of every call and reports a fixed load
/// for each.
pub(super) struct FakeMetrics {
    calls: SampledCalls,
}

impl FakeMetrics {
    /// A sampler recording its calls into `calls`.
    pub(super) fn new(calls: SampledCalls) -> Self {
        Self { calls }
    }
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
