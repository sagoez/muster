#[cfg(not(target_os = "linux"))]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::{fs, path::Path};

use muster_domain::value::{BootId, Pid, StartToken};

use crate::process::signal_target::SignalTarget;

/// The field of `/proc/<pid>/stat`, after the command name, holding the state.
#[cfg(target_os = "linux")]
const STAT_STATE: usize = 0;
/// The field of `/proc/<pid>/stat`, after the command name, holding the start
/// time in clock ticks since boot.
#[cfg(target_os = "linux")]
const STAT_START_TIME: usize = 19;
/// The states of a process that exited and only waits to be reaped: zombie
/// and dead.
#[cfg(target_os = "linux")]
const EXITED_STATES: &[&str] = &["Z", "X"];
/// The root of the per-process status folders.
#[cfg(target_os = "linux")]
const PROC_ROOT: &str = "/proc";
/// A process's status file under its folder.
#[cfg(target_os = "linux")]
const STAT_FILE: &str = "stat";
/// What ends the parenthesized command name in a stat line, which may itself
/// hold spaces and parentheses.
#[cfg(target_os = "linux")]
const COMMAND_END: &str = ") ";
/// The file naming this boot of the system.
#[cfg(target_os = "linux")]
const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";
/// The program that reports a process's start time where there is no
/// `/proc`.
#[cfg(not(target_os = "linux"))]
const PS_PROGRAM: &str = "ps";
/// The `ps` arguments that print only the start time of the process whose
/// pid follows.
#[cfg(not(target_os = "linux"))]
const PS_START_TIME_ARGS: [&str; 3] = ["-o", "lstart=", "-p"];

/// Offset basis for a deterministic process-start-text fingerprint.
#[cfg(not(target_os = "linux"))]
const START_TOKEN_HASH_OFFSET: u64 = 14_695_981_039_346_656_037;
/// Multiplier for a deterministic process-start-text fingerprint.
#[cfg(not(target_os = "linux"))]
const START_TOKEN_HASH_PRIME: u64 = 1_099_511_628_211;

/// Queries the operating system for a process's non-reusable creation marker.
pub struct LocalProcessIdentity;

impl LocalProcessIdentity {
    /// Returns the creation marker for `process_id` when this platform exposes one.
    pub fn start_token(process_id: Pid) -> Option<StartToken> {
        Self::platform_start_token(process_id)
    }

    /// Whether `process_id` is still the process that started with `expected`:
    /// it exists and its start token matches, which an unreadable token never
    /// disproves.
    pub fn is_same_process(process_id: Pid, expected: Option<StartToken>) -> bool {
        SignalTarget::Process(process_id).is_alive()
            && Self::same_start(expected, Self::start_token(process_id))
    }

    /// Whether a process whose start token reads `current` may be the one
    /// started with `recorded`: they differ only when both are known and
    /// unequal.
    pub fn same_start(recorded: Option<StartToken>, current: Option<StartToken>) -> bool {
        match (recorded, current) {
            (Some(recorded), Some(current)) => recorded == current,
            _ => true,
        }
    }

    /// This boot of the system, where a start token counts from boot and so
    /// may come round again after a reboot: a process recorded in another
    /// boot is gone, whatever now runs with its pid and token. None where a
    /// token never repeats across boots, or the boot cannot be read.
    pub fn boot_id() -> Option<BootId> {
        Self::platform_boot_id()
    }

    /// Whether a process recorded in the boot `recorded` may still run: it
    /// is this boot, or either boot is unknown.
    pub fn may_be_this_boot(recorded: Option<&BootId>) -> bool {
        recorded.is_none_or(|recorded| Self::boot_id().is_none_or(|boot| boot == *recorded))
    }

    /// The boot id the kernel draws at each boot.
    #[cfg(target_os = "linux")]
    fn platform_boot_id() -> Option<BootId> {
        BootId::try_new(fs::read_to_string(BOOT_ID_FILE).ok()?).ok()
    }

    /// None: `ps` reports a start as a wall-clock time, which a later boot
    /// does not repeat.
    #[cfg(not(target_os = "linux"))]
    fn platform_boot_id() -> Option<BootId> {
        None
    }

    /// Whether `process_id` has exited and only waits to be reaped, so it never
    /// runs again. Returns `false` when that cannot be read.
    pub fn has_exited(process_id: Pid) -> bool {
        Self::platform_has_exited(process_id)
    }

    /// Whether `process_id` is a zombie or dead, read from its `/proc` stat.
    #[cfg(target_os = "linux")]
    fn platform_has_exited(process_id: Pid) -> bool {
        stat_fields(process_id)
            .and_then(|fields| fields.into_iter().nth(STAT_STATE))
            .is_some_and(|state| EXITED_STATES.contains(&state.as_str()))
    }

    /// Never known elsewhere: a process is taken to run until it is gone.
    #[cfg(not(target_os = "linux"))]
    fn platform_has_exited(_process_id: Pid) -> bool {
        false
    }

    /// The start time `/proc` records for `process_id`, in clock ticks.
    #[cfg(target_os = "linux")]
    fn platform_start_token(process_id: Pid) -> Option<StartToken> {
        stat_fields(process_id)?
            .into_iter()
            .nth(STAT_START_TIME)?
            .parse::<u64>()
            .ok()
            .and_then(|token| StartToken::try_new(token).ok())
    }

    /// The start time `ps` reports for `process_id`.
    #[cfg(not(target_os = "linux"))]
    fn platform_start_token(process_id: Pid) -> Option<StartToken> {
        let output = Command::new(PS_PROGRAM)
            .args(PS_START_TIME_ARGS)
            .arg(process_id.into_inner().to_string())
            .output()
            .ok()?;
        let started = String::from_utf8(output.stdout).ok()?;
        let token = started
            .trim()
            .bytes()
            .fold(START_TOKEN_HASH_OFFSET, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(START_TOKEN_HASH_PRIME)
            });
        StartToken::try_new(token).ok()
    }
}

/// The fields of `/proc/<pid>/stat` after the command name, which may itself
/// hold spaces and parentheses.
#[cfg(target_os = "linux")]
fn stat_fields(process_id: Pid) -> Option<Vec<String>> {
    let path = Path::new(PROC_ROOT)
        .join(process_id.into_inner().to_string())
        .join(STAT_FILE);
    let stat = fs::read_to_string(path).ok()?;
    Some(
        stat.rsplit_once(COMMAND_END)?
            .1
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        process::{self, Command},
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    /// Bound on waiting for a child to exit.
    const EXIT_WAIT: Duration = Duration::from_secs(5);
    /// Interval between checks that a child exited.
    const EXIT_POLL: Duration = Duration::from_millis(5);

    /// A child that exited but was not yet reaped has exited, though its pid
    /// still exists; a running process has not.
    #[test]
    fn an_unreaped_child_has_exited() {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let started = Instant::now();
        while !LocalProcessIdentity::has_exited(pid) {
            assert!(started.elapsed() < EXIT_WAIT, "the child never exited");
            thread::sleep(EXIT_POLL);
        }

        assert!(SignalTarget::Process(pid).is_alive(), "still unreaped");
        child.wait().unwrap();
        assert!(!LocalProcessIdentity::has_exited(
            Pid::try_new(process::id()).unwrap()
        ));
    }

    /// This boot's id is read, and reads the same each time.
    #[test]
    fn the_boot_id_is_read_and_stable() {
        let boot = LocalProcessIdentity::boot_id().unwrap();

        assert_eq!(LocalProcessIdentity::boot_id(), Some(boot.clone()));
        assert!(LocalProcessIdentity::may_be_this_boot(Some(&boot)));
        assert!(LocalProcessIdentity::may_be_this_boot(None));
        let earlier = BootId::try_new(format!("{boot}-earlier")).unwrap();
        assert!(!LocalProcessIdentity::may_be_this_boot(Some(&earlier)));
    }
}
