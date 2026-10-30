use std::collections::{HashMap, HashSet};

use sysinfo::{Pid as SysinfoPid, System};

use crate::domain::value::Pid;

/// The parent-to-children links of every process `sysinfo` last refreshed,
/// for walking the tree under a supervised root.
pub struct ProcessTree {
    /// Each parent's direct children.
    children: HashMap<SysinfoPid, Vec<SysinfoPid>>,
}

impl ProcessTree {
    /// The tree of the processes `system` holds; their parents must have been
    /// refreshed, which every `sysinfo` process refresh does.
    pub fn of(system: &System) -> Self {
        let mut children: HashMap<SysinfoPid, Vec<SysinfoPid>> = HashMap::new();
        for (pid, process) in system.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        Self { children }
    }

    /// `root` and every process below it, each once even if the links loop.
    pub fn subtree(&self, root: Pid) -> Vec<SysinfoPid> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![SysinfoPid::from_u32(root.into_inner())];
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
