use std::collections::HashMap;

use getset::Getters;
use typed_builder::TypedBuilder;

/// A point-in-time resource-usage reading aggregated over a process subtree.
#[derive(Clone, Copy, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProcessSample {
    /// Total CPU use across the tree as a percentage, where 100 is one core.
    cpu_percent: f32,
    /// Resident memory across the tree, in bytes.
    memory_bytes: u64,
}

/// Samples resource usage for process trees. A driven port so the domain never
/// names a metrics library. Called on the main loop, so it carries no
/// thread-safety bound; `&mut self` lets an implementation keep the state its
/// CPU-delta accounting needs between samples.
pub trait ProcessMetrics {
    /// Returns a sample for each pid in `pids`, aggregated over that pid's process
    /// subtree. A pid with no live process is absent from the returned map.
    fn sample(&mut self, pids: &[u32]) -> HashMap<u32, ProcessSample>;
}
