use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::domain::port::{ProcessMetrics, ProcessSample};

/// A [`ProcessMetrics`] backed by `sysinfo`, aggregating CPU and memory over each
/// process subtree so a shell's real workload (its children) is counted, not just
/// the shell. CPU is delta-based: the first sample after a process appears reads
/// zero until a later refresh gives sysinfo a window to measure against.
pub struct SysinfoMetrics {
    system: System,
}

impl Default for SysinfoMetrics {
    fn default() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl ProcessMetrics for SysinfoMetrics {
    fn sample(&mut self, pids: &[u32]) -> HashMap<u32, ProcessSample> {
        // Nothing to measure: skip the scan entirely rather than refresh every
        // process on the host for no roots.
        if pids.is_empty() {
            return HashMap::new();
        }
        // Refresh only CPU and memory (the parent PID is always collected), so a
        // host with many processes is not enumerated for disk, exe, and command
        // data on the single TUI event loop. `without_tasks` is essential, not
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
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, process) in processes {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        let mut samples = HashMap::new();
        for &root in pids {
            let mut cpu_percent = 0.0_f32;
            let mut memory_bytes = 0_u64;
            let mut found = false;
            let mut seen = HashSet::new();
            let mut stack = vec![Pid::from(root as usize)];
            while let Some(pid) = stack.pop() {
                if !seen.insert(pid) {
                    continue;
                }
                if let Some(process) = processes.get(&pid) {
                    found = true;
                    cpu_percent += process.cpu_usage();
                    memory_bytes += process.memory();
                }
                if let Some(kids) = children.get(&pid) {
                    stack.extend(kids);
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
