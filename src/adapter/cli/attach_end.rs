use muster_domain::pty::ExitOutcome;

use super::ending_signal::EndingSignal;

/// How a `muster attach` session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachEnd {
    /// The person pressed the detach key, or the terminal closed its input;
    /// the process keeps running.
    Detached,
    /// The process exited.
    Exited(ExitOutcome),
    /// A signal ended the client; the process keeps running.
    Signalled(EndingSignal),
}
