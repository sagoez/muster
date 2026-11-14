use muster_domain::value::OutputCursor;
use muster_protocol::{holder::LogScan, service::WaitStatus};

use crate::RunPin;

/// The scan of the log a wait is held on while its cursor is older than what
/// its process's tap holds.
pub(crate) struct HeldScan {
    /// Where the tap's lines begin, which the scan stops at.
    pub(crate) until: OutputCursor,
    /// The runs the wait is about, fixed when it was held.
    pub(super) pin: RunPin,
    /// How its run ended while the scan was in flight, if it did.
    pub(super) ended: Option<WaitStatus>,
    /// What the scan found, once it landed; none when it could not be read.
    pub(crate) found: Option<LogScan>,
}
