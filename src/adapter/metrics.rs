use std::collections::HashMap;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, set_open_files_limit};

use super::process_tree::ProcessTree;
use crate::domain::{
    port::{ProcessMetrics, ProcessSample},
    value::Pid,
};

/// How many `/proc` files `sysinfo` may keep open between refreshes. It keeps
/// one per process it has seen otherwise, up to half the hard limit.
const SYSINFO_OPEN_FILES: usize = 0;

/// A [`ProcessMetrics`] backed by `sysinfo`, aggregating CPU and memory over each
/// process subtree so a shell's real workload (its children) is counted, not just
/// the shell. CPU is delta-based: the first sample after a process appears reads
/// zero until a later refresh gives sysinfo a window to measure against.
pub struct SysinfoMetrics {
    system: System,
}

impl Default for SysinfoMetrics {
    /// A sampler that keeps no `/proc` file open between samples. Building one
    /// raises this process's open-files soft limit to the hard limit, so a
    /// service reads its own limit first.
    fn default() -> Self {
        set_open_files_limit(SYSINFO_OPEN_FILES);
        Self {
            system: System::new(),
        }
    }
}

impl ProcessMetrics for SysinfoMetrics {
    fn sample(&mut self, pids: &[Pid]) -> HashMap<Pid, ProcessSample> {
        // Nothing to measure: skip the scan entirely rather than refresh every
        // process on the host for no roots.
        if pids.is_empty() {
            return HashMap::new();
        }
        // Refresh only CPU and memory (the parent PID is always collected), so a
        // host with many processes is not enumerated for disk, exe, and command
        // data on the service's event loop. `without_tasks` is essential, not
        // just cheaper: `nothing()` leaves task collection on, and on Linux each
        // thread would appear as a child in `processes()`, so summing the subtree
        // would add the process-wide RSS once per thread and inflate the total.
        // The subtree still needs every process's parent, hence `All`.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .without_tasks(),
        );
        let processes = self.system.processes();
        let tree = ProcessTree::of(&self.system);
        let mut samples = HashMap::new();
        for &root in pids {
            let mut cpu_percent = 0.0_f32;
            let mut memory_bytes = 0_u64;
            let mut found = false;
            for pid in tree.subtree(root) {
                if let Some(process) = processes.get(&pid) {
                    found = true;
                    cpu_percent += process.cpu_usage();
                    memory_bytes += process.memory();
                }
            }
            if found {
                samples.insert(
                    root,
                    ProcessSample::builder()
                        .cpu_percent(cpu_percent)
                        .memory_bytes(memory_bytes)
                        .build(),
                );
            }
        }
        samples
    }
}
