use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, System};

/// The parent-to-children links of every process `sysinfo` last refreshed,
/// for walking the tree under a supervised root.
pub struct ProcessTree {
    /// Each parent's direct children.
    children: HashMap<Pid, Vec<Pid>>,
}

impl ProcessTree {
    /// The tree of the processes `system` holds; their parents must have been
    /// refreshed, which every `sysinfo` process refresh does.
    pub fn of(system: &System) -> Self {
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, process) in system.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        Self { children }
    }

    /// `root` and every process below it, each once even if the links loop.
    pub fn subtree(&self, root: u32) -> Vec<Pid> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![Pid::from_u32(root)];
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            order.push(pid);
            if let Some(kids) = self.children.get(&pid) {
                stack.extend(kids);
            }
        }
        order
    }
}
