//! The hooked shell a muster command runs from: the shell its hook exported,
//! found among the command's ancestors as doctor finds it, or as its direct
//! parent for a registration.

use std::env;

use rustix::process::getpid;
use sysinfo::System;

use crate::{
    adapter::process_tree::{named_ancestors, working_dir},
    constants::MUSTER_SHELL_ENV,
    domain::value::{Pid, ProjectRoot},
};

/// Most of this process's ancestors searched for the hooked shell: past any
/// chain of wrappers it runs under.
const HOOK_SEARCH_DEPTH: usize = 16;
/// The ancestors a registration looks at: only its direct parent.
const PARENT_ONLY: usize = 1;
/// Names of the programs that are shells: the nearest of this process's
/// ancestors running one is the shell it runs in, and wrappers such as
/// `env`, `timeout` or `mise` between them are passed over.
const SHELL_NAMES: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ash", "ksh", "ksh93", "mksh", "oksh", "pdksh", "yash",
    "posh", "csh", "tcsh", "nu", "elvish", "xonsh", "pwsh", "osh", "ysh", "rc", "es",
];
/// What a login shell's name starts with.
const LOGIN_SHELL_MARK: char = '-';

/// The hooked shell this process runs straight from, its direct parent when
/// that is the shell its hook exported, while that shell works in `root` or
/// a folder inside it.
pub fn hooked_shell_in(root: &ProjectRoot) -> Option<Pid> {
    parent_hooked_shell(
        env::var(MUSTER_SHELL_ENV).ok().as_deref(),
        &ancestors_up_to(PARENT_ONLY),
    )
    .filter(|shell| working_dir(*shell).is_some_and(|dir| root.contains(&dir)))
}

/// This process's ancestors, nearest first, each a pid and its process
/// name.
pub(super) fn own_ancestors() -> Vec<(Pid, String)> {
    ancestors_up_to(HOOK_SEARCH_DEPTH)
}

/// At most `limit` of this process's ancestors, nearest first, each a pid
/// and its process name.
fn ancestors_up_to(limit: usize) -> Vec<(Pid, String)> {
    u32::try_from(getpid().as_raw_nonzero().get())
        .ok()
        .and_then(|this| Pid::try_new(this).ok())
        .map(|this| named_ancestors(&mut System::new(), this, limit))
        .unwrap_or_default()
}

/// The shell the hook exported as `exported`, only when it is the first of
/// `ancestors`, the direct parent: a program such as `npm` or `xargs` that
/// runs muster as its child passes no hooked shell on.
fn parent_hooked_shell(exported: Option<&str>, ancestors: &[(Pid, String)]) -> Option<Pid> {
    nearest_hooked_shell(exported, ancestors.get(..PARENT_ONLY).unwrap_or(ancestors))
}

/// The shell the hook exported as `exported`, when it is the nearest shell
/// among `ancestors`, each a pid and its process name: so under `env`,
/// `timeout` or `mise exec` it is still found, while a shell started inside
/// it, which inherits the exported pid, is not taken for it.
pub(super) fn nearest_hooked_shell(
    exported: Option<&str>,
    ancestors: &[(Pid, String)],
) -> Option<Pid> {
    let nearest = ancestors
        .iter()
        .find(|(_, name)| is_shell(name))
        .map(|(pid, _)| *pid)?;
    let exported = exported?.trim().parse::<u32>().ok()?;
    (nearest.into_inner() == exported).then_some(nearest)
}

/// Whether a process named `name` runs a shell, a login shell's leading
/// mark aside.
fn is_shell(name: &str) -> bool {
    SHELL_NAMES.contains(&name.trim_start_matches(LOGIN_SHELL_MARK))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ancestors `raw` names, each a pid and its process name.
    fn pids(raw: &[(u32, &str)]) -> Vec<(Pid, String)> {
        raw.iter()
            .map(|(pid, name)| (Pid::try_new(*pid).unwrap(), (*name).to_string()))
            .collect()
    }

    /// The exported shell is found only as the nearest shell, past wrappers;
    /// a shell inside it, a non-shell on its pid, or a pid that does not
    /// parse is not.
    #[test]
    fn the_hooked_shell_is_the_nearest_shell_it_exported() {
        let hooked = Pid::try_new(4242).unwrap();
        for ancestors in [&[(4242, "zsh"), (300, "systemd")][..], &[
            (4301, "timeout"),
            (4300, "mise"),
            (4242, "-bash"),
        ]] {
            assert_eq!(
                nearest_hooked_shell(Some("4242"), &pids(ancestors)),
                Some(hooked),
                "{ancestors:?}"
            );
        }
        for (exported, ancestors) in [
            (Some("4242"), &[(5000, "bash"), (4242, "bash")][..]),
            (Some("4242"), &[(4242, "python3"), (300, "systemd")]),
            (None, &[(4242, "zsh")]),
            (Some("not a pid"), &[(4242, "zsh")]),
            (Some("4242"), &[]),
        ] {
            assert_eq!(
                nearest_hooked_shell(exported, &pids(ancestors)),
                None,
                "{exported:?} {ancestors:?}"
            );
        }
    }

    /// A registration's hooked shell is only its direct parent: under a
    /// program that runs muster as its child, as `npm`, `xargs` or
    /// `timeout` do, there is none.
    #[test]
    fn a_registration_s_hooked_shell_is_its_direct_parent() {
        let hooked = Pid::try_new(4242).unwrap();
        for ancestors in [&[(4242, "zsh"), (300, "systemd")][..], &[(4242, "-bash")]] {
            assert_eq!(
                parent_hooked_shell(Some("4242"), &pids(ancestors)),
                Some(hooked),
                "{ancestors:?}"
            );
        }
        for ancestors in [
            &[(4301, "node"), (4300, "npm"), (4242, "bash")][..],
            &[(4301, "xargs"), (4242, "zsh")],
            &[(4301, "timeout"), (4242, "bash")],
            &[(5000, "bash"), (4242, "bash")],
            &[],
        ] {
            assert_eq!(
                parent_hooked_shell(Some("4242"), &pids(ancestors)),
                None,
                "{ancestors:?}"
            );
        }
    }

    /// A process is a shell by its name, a login shell's leading mark
    /// aside; wrappers and other programs are not.
    #[test]
    fn shells_are_told_by_name() {
        for shell in ["bash", "zsh", "fish", "sh", "dash", "ksh", "-bash", "-zsh"] {
            assert!(is_shell(shell), "{shell}");
        }
        for other in ["env", "timeout", "mise", "muster", "sudo", ""] {
            assert!(!is_shell(other), "{other}");
        }
    }
}
