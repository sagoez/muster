use crate::domain::value::{Pid, StartToken};

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
        Self::is_alive(process_id) && Self::same_start(expected, Self::start_token(process_id))
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

    /// Whether `process_id` currently exists. Returns `true` when liveness cannot be
    /// determined, such as a failed lookup, so a caller that guards on liveness
    /// does not fail open on uncertainty.
    pub fn is_alive(process_id: Pid) -> bool {
        Self::platform_is_alive(process_id)
    }

    #[cfg(target_os = "linux")]
    fn platform_is_alive(process_id: Pid) -> bool {
        std::path::Path::new(&format!("/proc/{}", process_id.into_inner())).exists()
    }

    #[cfg(not(target_os = "linux"))]
    fn platform_is_alive(process_id: Pid) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &process_id.into_inner().to_string()])
            .status()
            .map_or(true, |status| status.success())
    }

    #[cfg(target_os = "linux")]
    fn platform_start_token(process_id: Pid) -> Option<StartToken> {
        let stat =
            std::fs::read_to_string(format!("/proc/{}/stat", process_id.into_inner())).ok()?;
        let fields = stat
            .rsplit_once(") ")?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        fields
            .get(19)?
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
