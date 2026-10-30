use std::collections::HashMap;

use super::ProcessSample;
use crate::domain::value::Pid;

/// Samples resource usage for process trees. A driven port so the domain never
/// names a metrics library. Called on the main loop, so it carries no
/// thread-safety bound; `&mut self` lets an implementation keep the state its
/// CPU-delta accounting needs between samples.
pub trait ProcessMetrics {
    /// Returns a sample for each pid in `pids`, aggregated over that pid's process
    /// subtree. A pid with no live process is absent from the returned map.
    fn sample(&mut self, pids: &[Pid]) -> HashMap<Pid, ProcessSample>;
}
