use std::collections::{HashMap, HashSet};

use sysinfo::{Pid as SysinfoPid, Process, ProcessRefreshKind, ProcessesToUpdate, System};

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

/// The parent of `pid`, its parent and so on, at most `limit` of them,
/// each refreshed in `system` as it is reached.
pub fn ancestors(system: &mut System, pid: Pid, limit: usize) -> Vec<Pid> {
    named_ancestors(system, pid, limit)
        .into_iter()
        .map(|(ancestor, _)| ancestor)
        .collect()
}

/// [`ancestors`] of `pid`, each with its process name, empty when it does
/// not read.
pub fn named_ancestors(system: &mut System, pid: Pid, limit: usize) -> Vec<(Pid, String)> {
    let mut ancestors = Vec::new();
    let mut current = SysinfoPid::from_u32(pid.into_inner());
    refresh_one(system, current);
    while ancestors.len() < limit {
        let Some(parent) = system.process(current).and_then(Process::parent) else {
            break;
        };
        let Ok(parent_pid) = Pid::try_new(parent.as_u32()) else {
            break;
        };
        refresh_one(system, parent);
        let name = system
            .process(parent)
            .map(|process| process.name().to_string_lossy().into_owned())
            .unwrap_or_default();
        ancestors.push((parent_pid, name));
        current = parent;
    }
    ancestors
}

/// Reads `pid`'s entry, with its parent, into `system`.
fn refresh_one(system: &mut System, pid: SysinfoPid) {
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
}

#[cfg(test)]
mod tests {
    use std::process::{self, Command, Stdio};

    use super::*;

    /// A program that runs until it is killed.
    const SLEEPER: &str = "sleep";
    /// How long it would run, in seconds.
    const SLEEP_SECONDS: &str = "30";
    /// Ancestors any test walk may read.
    const LIMIT: usize = 64;

    /// A child's ancestors start with this test process, and the walk stops
    /// at its limit.
    #[test]
    fn a_child_s_ancestors_start_with_its_parent_and_stop_at_the_limit() {
        let mut child = Command::new(SLEEPER)
            .arg(SLEEP_SECONDS)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let this = Pid::try_new(process::id()).unwrap();

        let all = ancestors(&mut System::new(), pid, LIMIT);
        let one = ancestors(&mut System::new(), pid, 1);
        child.kill().unwrap();
        child.wait().unwrap();

        assert_eq!(all.first(), Some(&this), "{all:?}");
        assert_eq!(one, [this]);
    }

    /// Each ancestor comes with its process name.
    #[test]
    fn named_ancestors_name_each_process() {
        let mut child = Command::new(SLEEPER)
            .arg(SLEEP_SECONDS)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let this = Pid::try_new(process::id()).unwrap();

        let named = named_ancestors(&mut System::new(), pid, 1);
        child.kill().unwrap();
        child.wait().unwrap();

        let [(parent, name)] = &named[..] else {
            panic!("one ancestor: {named:?}");
        };
        assert_eq!(*parent, this);
        assert!(!name.is_empty());
    }
}
