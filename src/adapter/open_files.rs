use std::{os::unix::process::CommandExt, process::Command};

use nutype::nutype;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// A soft limit on open files, `None` when unlimited. The service records its
/// own before `sysinfo` raises it to the hard limit, and starts every holder
/// back at it, so a child runs with the limit the user gave muster.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub struct OpenFilesLimit(Option<u64>);

impl OpenFilesLimit {
    /// The calling process's current soft limit on open files.
    pub fn current() -> Self {
        Self::new(getrlimit(Resource::Nofile).current)
    }

    /// Makes `command` start with this soft limit, capped at its hard limit.
    pub fn restore_in(self, command: &mut Command) -> &mut Command {
        let soft = self.into_inner();
        // SAFETY: the hook runs between fork and exec and only makes the
        // `getrlimit` and `setrlimit` system calls, which are
        // async-signal-safe, allocate nothing and touch only stack values.
        unsafe {
            command.pre_exec(move || {
                let limits = getrlimit(Resource::Nofile);
                let current = match (soft, limits.maximum) {
                    (Some(soft), Some(hard)) => Some(soft.min(hard)),
                    (Some(soft), None) => Some(soft),
                    (None, hard) => hard,
                };
                setrlimit(Resource::Nofile, Rlimit {
                    current,
                    maximum: limits.maximum,
                })?;
                Ok(())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A soft limit below any hard limit a test host has.
    const LOWERED: Option<u64> = Some(64);
    /// Prints the shell's soft limit on open files.
    const PRINT_SOFT_LIMIT: &str = "ulimit -Sn";

    /// A command started under a recorded limit runs with it, whatever the
    /// spawning process's own soft limit is now.
    #[test]
    fn a_restored_command_runs_with_the_recorded_soft_limit() {
        let mut command = Command::new("sh");
        command.arg("-c").arg(PRINT_SOFT_LIMIT);

        let output = OpenFilesLimit::new(LOWERED)
            .restore_in(&mut command)
            .output()
            .unwrap();

        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            LOWERED.unwrap().to_string()
        );
    }

    /// The current limit reads the calling process's own soft limit.
    #[test]
    fn the_current_limit_is_the_soft_limit() {
        assert_eq!(
            OpenFilesLimit::current().into_inner(),
            getrlimit(Resource::Nofile).current
        );
    }
}
