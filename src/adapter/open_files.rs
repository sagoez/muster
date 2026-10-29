use std::{io, os::unix::process::CommandExt, process::Command};

use nutype::nutype;

/// A soft limit on open files. The service records its own before `sysinfo`
/// raises it to the hard limit, and starts every holder back at it, so a child
/// runs with the limit the user gave muster.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub struct OpenFilesLimit(libc::rlim_t);

impl OpenFilesLimit {
    /// The calling process's current soft limit on open files.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the limit cannot be read.
    pub fn current() -> io::Result<Self> {
        let limits = open_files_limits()?;
        Ok(Self::new(limits.rlim_cur))
    }

    /// Makes `command` start with this soft limit, capped at its hard limit.
    pub fn restore_in(self, command: &mut Command) -> &mut Command {
        let soft = self.into_inner();
        // SAFETY: the hook runs between fork and exec and only calls
        // `getrlimit` and `setrlimit`, which are async-signal-safe and touch
        // only the stack value they are given.
        unsafe {
            command.pre_exec(move || {
                let mut limits = open_files_limits()?;
                limits.rlim_cur = soft.min(limits.rlim_max);
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limits) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            })
        }
    }
}

/// The calling process's soft and hard limits on open files.
///
/// # Errors
/// Returns an [`io::Error`] when they cannot be read.
fn open_files_limits() -> io::Result<libc::rlimit> {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` only writes the struct it is handed.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A soft limit below any hard limit a test host has.
    const LOWERED: libc::rlim_t = 64;
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
            LOWERED.to_string()
        );
    }

    /// The current limit reads the calling process's own soft limit.
    #[test]
    fn the_current_limit_is_the_soft_limit() {
        let limits = open_files_limits().unwrap();

        assert_eq!(
            OpenFilesLimit::current().unwrap().into_inner(),
            limits.rlim_cur
        );
    }
}
