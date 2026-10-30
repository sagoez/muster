//! The hooked shell a muster command runs straight from: the shell its hook
//! exported, found among the command's ancestors as doctor finds it.

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
/// Names of the programs that are shells: the nearest of this process's
/// ancestors running one is the shell it runs in, and wrappers such as
/// `env`, `timeout` or `mise` between them are passed over.
const SHELL_NAMES: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ash", "ksh", "ksh93", "mksh", "oksh", "pdksh", "yash",
    "posh", "csh", "tcsh", "nu", "elvish", "xonsh", "pwsh", "osh", "ysh", "rc", "es",
];
/// What a login shell's name starts with.
const LOGIN_SHELL_MARK: char = '-';

/// The hooked shell this process runs straight from, as
/// [`nearest_hooked_shell`] finds it among its own ancestors, while that
/// shell works in `root` or a folder inside it.
pub fn hooked_shell_in(root: &ProjectRoot) -> Option<Pid> {
    nearest_hooked_shell(env::var(MUSTER_SHELL_ENV).ok().as_deref(), &own_ancestors())
        .filter(|shell| working_dir(*shell).is_some_and(|dir| root.contains(&dir)))
}

/// This process's ancestors, nearest first, each a pid and its process
/// name.
pub(super) fn own_ancestors() -> Vec<(Pid, String)> {
    u32::try_from(getpid().as_raw_nonzero().get())
        .ok()
        .and_then(|this| Pid::try_new(this).ok())
        .map(|this| named_ancestors(&mut System::new(), this, HOOK_SEARCH_DEPTH))
        .unwrap_or_default()
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
