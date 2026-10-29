#[cfg(target_os = "linux")]
use std::fs;

use crate::{
    adapter::signal_target::SignalTarget,
    domain::value::{Pid, StartToken},
};

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

    /// Whether `process_id` has exited and only waits to be reaped, so it never
    /// runs again. Returns `false` when that cannot be read.
    pub fn has_exited(process_id: Pid) -> bool {
        Self::platform_has_exited(process_id)
    }

    #[cfg(target_os = "linux")]
    fn platform_has_exited(process_id: Pid) -> bool {
        stat_fields(process_id)
            .and_then(|fields| fields.into_iter().nth(STAT_STATE))
            .is_some_and(|state| EXITED_STATES.contains(&state.as_str()))
    }

    #[cfg(not(target_os = "linux"))]
    fn platform_has_exited(_process_id: Pid) -> bool {
        false
    }

    #[cfg(target_os = "linux")]
    fn platform_start_token(process_id: Pid) -> Option<StartToken> {
        stat_fields(process_id)?
            .into_iter()
            .nth(STAT_START_TIME)?
            .parse::<u64>()
            .ok()
            .and_then(|token| StartToken::try_new(token).ok())
    }

    #[cfg(not(target_os = "linux"))]
    fn platform_start_token(process_id: Pid) -> Option<StartToken> {
        use std::process::Command;

        let output = Command::new("ps")
            .args(["-o", "lstart=", "-p", &process_id.into_inner().to_string()])
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
    let stat = fs::read_to_string(format!("/proc/{}/stat", process_id.into_inner())).ok()?;
    Some(
        stat.rsplit_once(") ")?
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
}
