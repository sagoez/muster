use serde::{Deserialize, Serialize};
use strum::Display;

use super::signal_name::SignalName;

/// Why a process ended without success.
#[derive(Debug, Clone, PartialEq, Eq, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum ExitCause {
    /// It exited with this non-zero status.
    #[strum(to_string = "exited with status {0}")]
    Status(u32),
    /// A signal ended it.
    #[strum(to_string = "ended by signal: {0}")]
    Signal(SignalName),
    /// Its end was observed without a status, as when the stream carrying its
    /// exit broke.
    #[strum(to_string = "ended without a status")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cause_reads_as_what_happened() {
        assert_eq!(ExitCause::Status(127).to_string(), "exited with status 127");
        assert_eq!(
            ExitCause::Signal(SignalName::try_new("sigkill").unwrap()).to_string(),
            "ended by signal: SIGKILL"
        );
        assert_eq!(ExitCause::Unknown.to_string(), "ended without a status");
    }
}
