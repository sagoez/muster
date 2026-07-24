use crate::domain::agent_session::{AgentProcessId, AgentProcessStartToken};

/// Offset basis for a deterministic process-start-text fingerprint.
#[cfg(all(unix, not(target_os = "linux")))]
const START_TOKEN_HASH_OFFSET: u64 = 14_695_981_039_346_656_037;
/// Multiplier for a deterministic process-start-text fingerprint.
#[cfg(all(unix, not(target_os = "linux")))]
const START_TOKEN_HASH_PRIME: u64 = 1_099_511_628_211;

/// Queries the operating system for a process's non-reusable creation marker.
pub struct LocalProcessIdentity;

impl LocalProcessIdentity {
    /// Returns the creation marker for `process_id` when this platform exposes one.
    pub fn start_token(process_id: AgentProcessId) -> Option<AgentProcessStartToken> {
        Self::platform_start_token(process_id)
    }

    #[cfg(target_os = "linux")]
    fn platform_start_token(process_id: AgentProcessId) -> Option<AgentProcessStartToken> {
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
            .and_then(|token| AgentProcessStartToken::try_new(token).ok())
    }

    #[cfg(windows)]
    fn platform_start_token(process_id: AgentProcessId) -> Option<AgentProcessStartToken> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };

        // SAFETY: the PID is an ordinary integer, the output FILETIMEs are valid
        // initialized stack storage, and the opened handle is closed on every path.
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                process_id.into_inner(),
            );
            if handle.is_null() {
                return None;
            }
            let mut created = std::mem::zeroed::<FILETIME>();
            let mut exited = std::mem::zeroed::<FILETIME>();
            let mut kernel = std::mem::zeroed::<FILETIME>();
            let mut user = std::mem::zeroed::<FILETIME>();
            let result = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
            let _ = CloseHandle(handle);
            (result != 0)
                .then(|| {
                    u64::from(created.dwLowDateTime) | (u64::from(created.dwHighDateTime) << 32)
                })
                .and_then(|token| AgentProcessStartToken::try_new(token).ok())
        }
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn platform_start_token(process_id: AgentProcessId) -> Option<AgentProcessStartToken> {
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
        AgentProcessStartToken::try_new(token).ok()
    }

    #[cfg(not(any(unix, windows)))]
    fn platform_start_token(_process_id: AgentProcessId) -> Option<AgentProcessStartToken> {
        None
    }
}
